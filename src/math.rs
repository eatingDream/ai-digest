use std::{iter::Peekable, str::Chars};

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn escaped(text: &str, i: usize) -> bool {
    text.as_bytes()[..i]
        .iter()
        .rev()
        .take_while(|&&c| c == b'\\')
        .count()
        % 2
        != 0
}

/// Unterminated delimiters stay prose. Return byte ranges including both delimiters.
pub(crate) fn math_ranges(text: &str) -> Vec<(std::ops::Range<usize>, &str)> {
    let mut ranges = Vec::new();
    let mut pos = 0;
    while let Some(offset) = text[pos..].find(['$', '\\']) {
        let start = pos + offset;
        let rest = &text[start..];
        let dollar = rest.starts_with('$');
        let width = if dollar { 1 } else { 2 };
        pos = start + 1;
        if escaped(text, start) || (!dollar && !rest.starts_with("\\(")) {
            continue;
        }
        let tail = &rest[width..];
        if dollar
            && (text[..start].ends_with('$')
                || tail
                    .chars()
                    .next()
                    .is_none_or(|c| c.is_whitespace() || c == '$'))
        {
            continue;
        }
        let line = tail.split(['\n', '\r']).next().unwrap_or("");
        let closing = if dollar { "$" } else { "\\)" };
        let close = line.match_indices(closing).find_map(|(i, _)| {
            let absolute = start + width + i;
            if escaped(text, absolute) {
                return None;
            }
            if dollar
                && (i == 0
                    || line[..i].ends_with(char::is_whitespace)
                    || line[i + 1..].starts_with('$')
                    || line[..i].ends_with('$')
                    || line.as_bytes().get(i + 1).is_some_and(u8::is_ascii_digit))
            {
                return None;
            }
            Some(i)
        });
        if let Some(close) = close {
            // An intervening dollar is its own opener, as in currency followed by math.
            if dollar
                && line[..close]
                    .match_indices('$')
                    .any(|(i, _)| !escaped(text, start + width + i))
            {
                continue;
            }
            pos = start + width + close + width;
            ranges.push((start..pos, &line[..close]));
        }
    }
    ranges
}

pub fn render_inline_math(text: &str) -> String {
    let mut out = String::new();
    let mut pos = 0;
    for (range, tex) in math_ranges(text) {
        out.push_str(&escape(&text[pos..range.start]));
        out.push_str(&render_formula(&text[range.clone()], tex));
        pos = range.end;
    }
    out.push_str(&escape(&text[pos..]));
    out
}

fn render_formula(original: &str, tex: &str) -> String {
    let mut parser = Parser {
        chars: tex.chars().peekable(),
        groups: 0,
    };
    match parser.sequence(0, false).filter(|s| !s.trim().is_empty()) {
        Some(html) => {
            let wrap = if tex.chars().count() > 24 {
                " math-wrap"
            } else {
                ""
            };
            format!(
                r#"<span class="math{wrap}" role="math" aria-label="{}">{html}</span>"#,
                escape(tex)
            )
        }
        None => format!(r#"<code class="math-raw">{}</code>"#, escape(original)),
    }
}

struct Parser<'a> {
    chars: Peekable<Chars<'a>>,
    groups: u8,
}

impl Parser<'_> {
    fn sequence(&mut self, depth: u8, grouped: bool) -> Option<String> {
        if self.groups >= 64 {
            return None;
        }
        self.groups += 1;
        let mut result = String::new();
        let mut base = false;
        let mut sub = false;
        let mut sup = false;
        while let Some(c) = self.chars.next() {
            match c {
                '}' if grouped && !result.trim().is_empty() => {
                    self.groups -= 1;
                    return Some(result);
                }
                '}' => return None,
                '_' | '^' => {
                    if !base || depth >= 2 || (c == '_' && sub) || (c == '^' && sup) {
                        return None;
                    }
                    let tag = if c == '_' {
                        sub = true;
                        "sub"
                    } else {
                        sup = true;
                        "sup"
                    };
                    let next = self.chars.next()?;
                    if next.is_whitespace() {
                        return None;
                    }
                    let body = self.atom(next, depth + 1)?;
                    result.push_str(&format!("<{tag}>{body}</{tag}>"));
                }
                _ => {
                    result.push_str(&self.atom(c, depth)?);
                    if !c.is_whitespace() {
                        base = true;
                        sub = false;
                        sup = false;
                    }
                }
            }
        }
        self.groups -= 1;
        (!grouped).then_some(result)
    }

    fn atom(&mut self, c: char, depth: u8) -> Option<String> {
        Some(match c {
            '{' => self.sequence(depth, true)?,
            '\\' => self.command(depth)?,
            '_' | '^' | '}' | '$' => return None,
            c if c.is_ascii_alphabetic() => format!("<i>{c}</i>"),
            c if c.is_ascii_digit() || c.is_whitespace() || "()[]+-=<>/,;:!|.*'&".contains(c) => {
                escape(&c.to_string())
            }
            _ => return None,
        })
    }

    fn group(&mut self, depth: u8) -> Option<String> {
        (self.chars.next()? == '{')
            .then(|| self.sequence(depth, true))
            .flatten()
    }

    fn literal_group(&mut self) -> Option<String> {
        if self.chars.next()? != '{' {
            return None;
        }
        let mut out = String::new();
        let mut nested = 0usize;
        while let Some(c) = self.chars.next() {
            match c {
                '}' if nested == 0 => return (!out.trim().is_empty()).then_some(out),
                '{' => nested += 1,
                '}' => nested -= 1,
                '\\' => {
                    let escaped = self.chars.next()?;
                    if !r"{}\$%&#_".contains(escaped) {
                        return None;
                    }
                    out.push(escaped);
                    continue;
                }
                _ => {}
            }
            out.push(c);
        }
        None
    }

    fn command(&mut self, depth: u8) -> Option<String> {
        let first = self.chars.next()?;
        if !first.is_ascii_alphabetic() {
            return match first {
                ',' | ':' | ';' | ' ' => Some(" ".into()),
                '{' | '}' | '$' | '_' | '%' | '&' | '#' => Some(escape(&first.to_string())),
                _ => None,
            };
        }
        let mut name = String::from(first);
        while let Some(next) = self.chars.next_if(char::is_ascii_alphabetic) {
            name.push(next);
        }
        self.chars.next_if(|c| *c == ' ');
        if let Some((_, letter, italic)) = GREEK.iter().find(|g| g.0 == name) {
            return Some(if *italic {
                format!("<i>{letter}</i>")
            } else {
                (*letter).into()
            });
        }
        Some(match name.as_str() {
            "exp" | "log" | "ln" | "sin" | "cos" | "max" | "min" | "lim" | "Pr" | "poly"
            | "det" => name,
            "le" | "leq" => "≤".into(),
            "ge" | "geq" => "≥".into(),
            "ne" | "neq" => "≠".into(),
            "approx" => "≈".into(),
            "sim" => "∼".into(),
            "cdot" => "·".into(),
            "times" => "×".into(),
            "pm" => "±".into(),
            "to" | "rightarrow" => "→".into(),
            "infty" => "∞".into(),
            "in" => "∈".into(),
            "sum" => "∑".into(),
            "prod" => "∏".into(),
            "partial" => "∂".into(),
            "nabla" => "∇".into(),
            "ldots" => "…".into(),
            "cdots" => "⋯".into(),
            "mid" => "|".into(),
            "frac" => format!(
                r#"<span class="frac">{}⁄{}</span>"#,
                self.group(depth)?,
                self.group(depth)?
            ),
            "sqrt" => format!(r#"√<span class="sqrt">{}</span>"#, self.group(depth)?),
            "text" => escape(&self.literal_group()?),
            "mathbb" => match self.literal_group()?.as_str() {
                "N" => "ℕ",
                "Z" => "ℤ",
                "Q" => "ℚ",
                "R" => "ℝ",
                "C" => "ℂ",
                _ => return None,
            }
            .into(),
            "mathcal" => {
                let group = self.literal_group()?;
                if !group.bytes().all(|c| c.is_ascii_uppercase()) {
                    return None;
                }
                group
                    .bytes()
                    .map(|c| {
                        "𝒜ℬ𝒞𝒟ℰℱ𝒢ℋℐ𝒥𝒦ℒℳ𝒩𝒪𝒫𝒬ℛ𝒮𝒯𝒰𝒱𝒲𝒳𝒴𝒵"
                            .chars()
                            .nth((c - b'A') as usize)
                            .unwrap()
                    })
                    .collect()
            }
            _ => return None,
        })
    }
}

const GREEK: &[(&str, &str, bool)] = &[
    ("alpha", "α", true),
    ("beta", "β", true),
    ("gamma", "γ", true),
    ("delta", "δ", true),
    ("epsilon", "ε", true),
    ("zeta", "ζ", true),
    ("eta", "η", true),
    ("theta", "θ", true),
    ("iota", "ι", true),
    ("kappa", "κ", true),
    ("lambda", "λ", true),
    ("mu", "μ", true),
    ("nu", "ν", true),
    ("xi", "ξ", true),
    ("omicron", "ο", true),
    ("pi", "π", true),
    ("rho", "ρ", true),
    ("sigma", "σ", true),
    ("tau", "τ", true),
    ("upsilon", "υ", true),
    ("phi", "φ", true),
    ("chi", "χ", true),
    ("psi", "ψ", true),
    ("omega", "ω", true),
    ("varepsilon", "ϵ", true),
    ("varphi", "ϕ", true),
    ("vartheta", "ϑ", true),
    ("Alpha", "Α", false),
    ("Beta", "Β", false),
    ("Gamma", "Γ", false),
    ("Delta", "Δ", false),
    ("Epsilon", "Ε", false),
    ("Zeta", "Ζ", false),
    ("Eta", "Η", false),
    ("Theta", "Θ", false),
    ("Iota", "Ι", false),
    ("Kappa", "Κ", false),
    ("Lambda", "Λ", false),
    ("Mu", "Μ", false),
    ("Nu", "Ν", false),
    ("Xi", "Ξ", false),
    ("Omicron", "Ο", false),
    ("Pi", "Π", false),
    ("Rho", "Ρ", false),
    ("Sigma", "Σ", false),
    ("Tau", "Τ", false),
    ("Upsilon", "Υ", false),
    ("Phi", "Φ", false),
    ("Chi", "Χ", false),
    ("Psi", "Ψ", false),
    ("Omega", "Ω", false),
];
