/// 提取 CF_HTML 中真正用于展示的 HTML 片段。
pub fn fragment(input: &str) -> &str {
    let header_end = cf_html_header_end(input);

    if let Some(header_end) = header_end {
        let start = header_value(input, header_end, "StartFragment:");
        let end = header_value(input, header_end, "EndFragment:");
        if let (Some(start), Some(end)) = (start, end)
            && let Some(fragment) = slice_by_offsets(input, start, end)
        {
            return marker_fragment(fragment).unwrap_or(fragment);
        }
    }

    if let Some(fragment) = marker_fragment(input) {
        return fragment;
    }

    // 偏移头损坏时仍去除头部，尽量让后续 HTML 解析有机会恢复内容。
    header_end.map_or(input, |header_end| &input[header_end..])
}

/// 按可见字符数裁剪 HTML，同时保留标签结构，供列表预览使用。
///
/// 裁剪只影响返回的预览字符串，不会修改剪贴板原始数据。
pub fn truncate_for_preview(input: &str, max_visible_chars: usize) -> String {
    const MAX_OUTPUT_BYTES: usize = 64 * 1024;
    const SUFFIX: &str = "...(内容过长已截断)";

    let source = fragment(input);
    if source.is_empty() {
        return String::new();
    }
    if max_visible_chars == 0 {
        return SUFFIX.to_owned();
    }

    let mut output = String::with_capacity(source.len().min(MAX_OUTPUT_BYTES));
    let mut open_tags = Vec::new();
    let mut visible_chars = 0usize;
    let output_needs_bound = source.len() > MAX_OUTPUT_BYTES;
    let mut truncated = false;
    let mut cursor = 0usize;

    while cursor < source.len() {
        if source.as_bytes()[cursor] == b'<' {
            if source[cursor..].starts_with("<!--") {
                let Some(end) = source[cursor + 4..].find("-->") else {
                    truncated = true;
                    break;
                };
                cursor += end + 7;
                continue;
            }

            let Some(end) = find_tag_end(source, cursor) else {
                truncated = true;
                break;
            };
            let raw_tag = &source[cursor..=end];
            let tag = parse_tag(raw_tag);
            if let Some(tag) = tag {
                if tag.is_ignored_element && !tag.is_closing && !tag.is_self_closing {
                    if append_tag(
                        &mut output,
                        &mut open_tags,
                        raw_tag,
                        &tag.name,
                        tag.is_closing,
                        tag.is_self_closing,
                        MAX_OUTPUT_BYTES,
                        SUFFIX,
                    ) {
                        if let Some(close_start) = find_closing_tag(source, end + 1, &tag.name) {
                            let close_end =
                                find_tag_end(source, close_start).unwrap_or(close_start);
                            let close_tag = &source[close_start..=close_end];
                            if !append_tag(
                                &mut output,
                                &mut open_tags,
                                close_tag,
                                &tag.name,
                                true,
                                false,
                                MAX_OUTPUT_BYTES,
                                SUFFIX,
                            ) {
                                truncated = true;
                                break;
                            }
                            cursor = close_end.saturating_add(1);
                            continue;
                        }
                    }
                    truncated = true;
                    break;
                }

                if !append_tag(
                    &mut output,
                    &mut open_tags,
                    raw_tag,
                    &tag.name,
                    tag.is_closing,
                    tag.is_self_closing,
                    MAX_OUTPUT_BYTES,
                    SUFFIX,
                ) {
                    truncated = true;
                }
                cursor = end.saturating_add(1);
                if truncated {
                    break;
                }
                continue;
            }

            // 非标签的尖括号按文本处理，避免异常 HTML 让裁剪器失去后续内容。
        }

        let mut next_tag = source[cursor..]
            .find('<')
            .map(|offset| cursor + offset)
            .unwrap_or(source.len());
        if next_tag == cursor {
            next_tag = cursor.saturating_add(1);
        }
        for character in source[cursor..next_tag].chars() {
            visible_chars += 1;
            if visible_chars > max_visible_chars {
                truncated = true;
                break;
            }
            let mut encoded = [0; 4];
            let character = character.encode_utf8(&mut encoded);
            let reserved = SUFFIX.len()
                + open_tags
                    .iter()
                    .map(|tag: &String| tag.len() + 3)
                    .sum::<usize>();
            if output
                .len()
                .saturating_add(character.len())
                .saturating_add(reserved)
                > MAX_OUTPUT_BYTES
            {
                truncated = true;
                break;
            }
            output.push_str(character);
        }
        if truncated {
            break;
        }
        cursor = next_tag;
    }

    if !truncated && !output_needs_bound {
        return source.to_owned();
    }

    for tag in open_tags.into_iter().rev() {
        let closing = format!("</{tag}>");
        if output
            .len()
            .saturating_add(closing.len())
            .saturating_add(SUFFIX.len())
            > MAX_OUTPUT_BYTES
        {
            break;
        }
        output.push_str(&closing);
    }
    if output.len().saturating_add(SUFFIX.len()) <= MAX_OUTPUT_BYTES {
        output.push_str(SUFFIX);
    }
    output
}

struct ParsedTag {
    name: String,
    is_closing: bool,
    is_self_closing: bool,
    is_ignored_element: bool,
}

fn find_tag_end(source: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    for (offset, character) in source[start + 1..].char_indices() {
        match (quote, character) {
            (Some(expected), character) if character == expected => quote = None,
            (None, '\'' | '"') => quote = Some(character),
            (None, '>') => return Some(start + 1 + offset),
            _ => {}
        }
    }
    None
}

fn parse_tag(raw_tag: &str) -> Option<ParsedTag> {
    let inner = raw_tag.strip_prefix('<')?.strip_suffix('>')?.trim();
    if inner.is_empty() || inner.starts_with('!') || inner.starts_with('?') {
        return None;
    }
    let is_closing = inner.starts_with('/');
    let name_start = usize::from(is_closing);
    let name = inner[name_start..]
        .split(|character: char| character.is_ascii_whitespace() || character == '/')
        .next()
        .filter(|name| !name.is_empty())?
        .to_ascii_lowercase();
    let is_self_closing = inner.ends_with('/')
        || matches!(
            name.as_str(),
            "area"
                | "base"
                | "br"
                | "col"
                | "embed"
                | "hr"
                | "img"
                | "input"
                | "link"
                | "meta"
                | "param"
                | "source"
                | "track"
                | "wbr"
        );
    Some(ParsedTag {
        is_ignored_element: matches!(name.as_str(), "script" | "style"),
        name,
        is_closing,
        is_self_closing,
    })
}

fn find_closing_tag(source: &str, start: usize, name: &str) -> Option<usize> {
    let needle = format!("</{name}");
    let mut cursor = start;
    while let Some(offset) = source[cursor..].find("</") {
        let candidate = cursor + offset;
        let after_name = candidate + needle.len();
        if source
            .get(candidate..after_name)
            .is_some_and(|value| value.eq_ignore_ascii_case(&needle))
            && source[after_name..]
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_whitespace() || character == '>')
        {
            return Some(candidate);
        }
        cursor = after_name;
    }
    None
}

fn append_tag(
    output: &mut String,
    open_tags: &mut Vec<String>,
    raw_tag: &str,
    name: &str,
    is_closing: bool,
    is_self_closing: bool,
    max_output_bytes: usize,
    suffix: &str,
) -> bool {
    let closing_position = is_closing.then(|| open_tags.iter().rposition(|tag| tag == name));
    let closing_position = closing_position.flatten();
    let reserved_open_tags = open_tags
        .iter()
        .map(|tag| tag.len() + 3)
        .sum::<usize>()
        .saturating_sub(
            closing_position
                .map(|position| open_tags[position].len() + 3)
                .unwrap_or(0),
        );
    let reserved = suffix.len()
        + reserved_open_tags
        + (!is_closing && !is_self_closing)
            .then_some(name.len() + 3)
            .unwrap_or(0);
    if output
        .len()
        .saturating_add(raw_tag.len())
        .saturating_add(reserved)
        > max_output_bytes
    {
        return false;
    }
    if let Some(position) = closing_position {
        open_tags.remove(position);
    }
    output.push_str(raw_tag);
    if !is_closing && !is_self_closing {
        open_tags.push(name.to_owned());
    }
    true
}

fn cf_html_header_end(input: &str) -> Option<usize> {
    let header_end = input.find('<')?;
    let header = &input[..header_end];
    if header.lines().any(|line| {
        line.trim_start_matches('\u{feff}')
            .trim()
            .starts_with("Version:")
    }) && header
        .lines()
        .any(|line| line.trim_start().starts_with("StartHTML:"))
    {
        Some(header_end)
    } else {
        None
    }
}

fn header_value(input: &str, header_end: usize, key: &str) -> Option<usize> {
    input[..header_end]
        .lines()
        .find_map(|line| line.strip_prefix(key)?.trim().parse().ok())
}

fn slice_by_offsets(input: &str, start: usize, end: usize) -> Option<&str> {
    (start <= end && end <= input.len())
        .then(|| input.get(start..end))
        .flatten()
}

fn marker_fragment(input: &str) -> Option<&str> {
    const START: &str = "<!--StartFragment-->";
    const END: &str = "<!--EndFragment-->";
    input
        .split_once(START)
        .and_then(|(_, rest)| rest.split_once(END).map(|(fragment, _)| fragment))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cf_html_offsets_extract_fragment() {
        let body =
            "<html><body><!--StartFragment--><p>彩色文本</p><!--EndFragment--></body></html>";
        let template = concat!(
            "Version:0.9\r\n",
            "StartHTML:0000000000\r\n",
            "EndHTML:0000000000\r\n",
            "StartFragment:0000000000\r\n",
            "EndFragment:0000000000\r\n",
        );
        let start_html = template.len();
        let start_fragment =
            start_html + body.find("<!--StartFragment-->").unwrap() + "<!--StartFragment-->".len();
        let end_fragment = start_html + body.find("<!--EndFragment-->").unwrap();
        let input = format!(
            "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\nStartFragment:{start_fragment:010}\r\nEndFragment:{end_fragment:010}\r\n{body}",
            end_html = start_html + body.len(),
        );

        assert_eq!(fragment(&input), "<p>彩色文本</p>");
    }

    #[test]
    fn fragment_markers_are_still_supported() {
        assert_eq!(
            fragment("<html><!--StartFragment--><b>内容</b><!--EndFragment--></html>"),
            "<b>内容</b>"
        );
    }

    #[test]
    fn non_cf_html_is_returned_unchanged() {
        assert_eq!(fragment("<p>普通 HTML</p>"), "<p>普通 HTML</p>");
    }
}
