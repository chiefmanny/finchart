//! Convert ANSI-colored terminal output (as produced by `render_chart` in
//! `ColorMode::Ansi`) into self-contained HTML for inspection in a browser.
//!
//! The renderer only emits green (32), red (31), and reset (0) codes, so the
//! converter handles just those.

/// Convert ANSI-colored text into an HTML fragment: green/red escapes become
/// `<span class="up|down">` elements; everything else is HTML-escaped.
pub fn ansi_to_html(ansi: &str) -> String {
    let mut out = String::with_capacity(ansi.len() * 2);
    let mut chars = ansi.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            push_escaped(&mut out, c);
            continue;
        }
        // Escape sequence: ESC '[' code 'm'. Collect the code digits.
        if chars.peek() == Some(&'[') {
            chars.next();
        }
        let mut code = String::new();
        for d in chars.by_ref() {
            if d == 'm' {
                break;
            }
            code.push(d);
        }
        match code.as_str() {
            "32" => out.push_str("<span class=\"up\">"),
            "31" => out.push_str("<span class=\"down\">"),
            "0" => out.push_str("</span>"),
            _ => {}
        }
    }
    out
}

/// Wrap ANSI-colored text in a self-contained dark-background monospace HTML
/// page that preserves spacing and renders up/down colors. No external assets.
pub fn to_document(ansi: &str) -> String {
    let body = ansi_to_html(ansi);
    format!(
        "<!DOCTYPE html>\n\
<html lang=\"en\">\n\
<head>\n\
<meta charset=\"utf-8\">\n\
<title>finchart snapshot</title>\n\
<style>\n\
  body {{ background: #0d1117; margin: 0; }}\n\
  pre {{\n\
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;\n\
    font-size: 12px; line-height: 1.15; color: #c9d1d9;\n\
    padding: 16px; white-space: pre;\n\
  }}\n\
  .up {{ color: #26a269; }}\n\
  .down {{ color: #e01b24; }}\n\
</style>\n\
</head>\n\
<body>\n\
<pre>{body}</pre>\n\
</body>\n\
</html>\n"
    )
}

fn push_escaped(out: &mut String, c: char) {
    match c {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        _ => out.push(c),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn green_becomes_up_span() {
        assert_eq!(
            ansi_to_html("\u{1b}[32m█\u{1b}[0m"),
            "<span class=\"up\">█</span>"
        );
    }

    #[test]
    fn red_becomes_down_span() {
        assert_eq!(
            ansi_to_html("\u{1b}[31m▒\u{1b}[0m"),
            "<span class=\"down\">▒</span>"
        );
    }

    #[test]
    fn special_characters_are_escaped() {
        assert_eq!(ansi_to_html("a<b>&c"), "a&lt;b&gt;&amp;c");
    }

    #[test]
    fn document_is_self_contained_and_preformatted() {
        let doc = to_document("\u{1b}[32m█\u{1b}[0m up \u{1b}[31m▒\u{1b}[0m down");
        assert!(doc.contains("<!DOCTYPE html>"), "missing doctype");
        assert!(doc.contains("<pre"), "missing <pre>");
        assert!(doc.contains("white-space: pre"), "spacing not preserved");
        assert!(doc.contains("class=\"up\"") && doc.contains("class=\"down\""));
        assert!(
            !doc.contains("http://") && !doc.contains("https://"),
            "external asset"
        );
        assert!(!doc.contains('\u{1b}'), "raw escape leaked");
    }
}
