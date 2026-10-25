const MAX_CHARS: usize = 1500;

pub fn speakable(md: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    for line in md.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if !in_code {
                out.push_str("(bloque de código).\n");
            }
            in_code = !in_code;
            continue;
        }
        if in_code || (trimmed.starts_with('|') && trimmed.ends_with('|')) {
            continue;
        }
        let body = strip_prefix(trimmed);
        if body.is_empty() {
            out.push_str(".\n");
            continue;
        }
        out.push_str(&inline(body));
        out.push('\n');
    }
    let mut text = collapse(&out);
    if text.chars().count() > MAX_CHARS {
        let cut: String = text.chars().take(MAX_CHARS).collect();
        let end = cut.rfind('.').unwrap_or(cut.len());
        text = format!("{}. El resto está en pantalla.", &cut[..end]);
    }
    text
}

fn strip_prefix(line: &str) -> &str {
    let l = line.trim_start_matches(['#', '>']).trim_start();
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = l.strip_prefix(marker) {
            return rest;
        }
    }
    let digits = l.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0
        && let Some(rest) = l[digits..].strip_prefix(". ")
    {
        return rest;
    }
    l
}

fn inline(s: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '!' && chars.get(i + 1) == Some(&'[') {
            i += 1;
            continue;
        }
        if c == '['
            && let Some(close) = chars[i..].iter().position(|&x| x == ']').map(|p| p + i)
            && chars.get(close + 1) == Some(&'(')
            && let Some(end) = chars[close..]
                .iter()
                .position(|&x| x == ')')
                .map(|p| p + close)
        {
            out.extend(&chars[i + 1..close]);
            i = end + 1;
            continue;
        }
        if s[byte_index(&chars, i)..].starts_with("http://")
            || s[byte_index(&chars, i)..].starts_with("https://")
        {
            out.push_str("un enlace");
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
            continue;
        }
        if !matches!(c, '*' | '_' | '`' | '~' | '#') {
            out.push(c);
        }
        i += 1;
    }
    out
}

fn byte_index(chars: &[char], i: usize) -> usize {
    chars[..i].iter().map(|c| c.len_utf8()).sum()
}

fn collapse(s: &str) -> String {
    let mut out = String::new();
    for line in s.lines() {
        if line == "." && (out.is_empty() || out.ends_with(".\n")) {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().trim_end_matches('.').trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_markdown() {
        let md = "# Hola\n\n**Ya** puedo `hablar`.\n\n```sh\nls\n```\n- uno\n- [dos](https://x.y)\n| a | b |";
        assert_eq!(
            speakable(md),
            "Hola\n.\nYa puedo hablar.\n(bloque de código).\nuno\ndos"
        );
    }

    #[test]
    fn names_urls() {
        assert_eq!(
            speakable("mira https://ejemplo.com ya"),
            "mira un enlace ya"
        );
    }

    #[test]
    fn trims_long_replies() {
        let long = "Frase corta. ".repeat(300);
        let out = speakable(&long);
        assert!(out.ends_with("El resto está en pantalla."));
        assert!(out.chars().count() < MAX_CHARS + 40);
    }
}
