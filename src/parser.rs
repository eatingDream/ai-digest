use std::{
    collections::{BTreeMap, HashSet},
    sync::LazyLock,
};

use anyhow::{ensure, Context, Result};
use chrono::{Datelike, NaiveDate};
use html5gum::Token;
use pulldown_cmark::{Event, Options as MdOptions, Parser as MdParser, Tag, TagEnd};
use regex::Regex;

use crate::{
    i18n::{self, Lang},
    model::{
        clean_text, html_tokens, Digest, Item, LinkItem, MustItem, ParsedIssue, ParsedItem,
        ParsedMust, ParsedSection, Section, SectionEntry, ThemeItem, TocEntry, TocGroup,
    },
};

static ITEM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^###\s+(🆕\s+)?([0-9]+(?:[–-][0-9]+)?)\.\s+(.*)$").unwrap());
static FIELD: LazyLock<Regex> = LazyLock::new(|| {
    let labels = i18n::LABEL_MAP
        .iter()
        .map(|(k, _)| regex::escape(k))
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&format!(r"^\*\*({labels})\s*[：:]\*\*\s*(.*)$")).unwrap()
});
static SHORT_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([0-9]{1,2})/([0-9]{1,2})").unwrap());
static LEAD_EMOJI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[\x{1F000}-\x{1FAFF}\u2190-\u21FF\u2300-\u27BF\u2B00-\u2BFF\uFE0F\u200D\s]+")
        .unwrap()
});
static TOC_SPLIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\u3000+|\s{2,}").unwrap());
static TOC_ENTRY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(🆕\s*)?([0-9]+(?:[–-][0-9]+)?)\.\s*(.+)$").unwrap());
static LIST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:[0-9]+[.、)]|[-*+])\s+").unwrap());
static MUST_BOLD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)^\*\*(.+?)\*\*\s*(?:[（(]([^）)]+)[）)])?\s*[：:]\s*(.*)$").unwrap()
});
static MUST_PLAIN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)^(.+?)[：:]\s*(.*)$").unwrap());
static PARENS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[（(][^）)]+[）)]").unwrap());
static ANCHOR_SPLIT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[与：:]|\s+(?:and|&)\s+").unwrap());
static ANCHOR_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z][A-Za-z0-9.\-_+]{3,}|[\u4e00-\u9fff]{3,}").unwrap());

pub fn parse_digest_md(md: &str, lang: Lang, fallback_year: Option<i32>) -> Result<Digest> {
    let input = md.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = input
        .lines()
        .take_while(|line| !i18n::DROPPED_MARKERS.contains(&line.trim()))
        .skip_while(|line| line.trim().is_empty())
        .peekable();
    let raw_title = lines
        .next()
        .context("empty markdown")?
        .trim()
        .trim_start_matches('#')
        .trim()
        .trim_matches('*')
        .trim();
    let (title, period) = match raw_title.rsplit_once('·') {
        Some((head, tail)) if tail.chars().any(|c| c.is_ascii_digit()) => (head, tail),
        Some((head, tail)) if !i18n::looks_weekly(tail) => (head, tail),
        _ => (raw_title, ""),
    };
    let weekly = i18n::looks_weekly(title);
    let mut digest = Digest {
        lang,
        title: title.trim().into(),
        period: period.trim().into(),
        weekly,
        dates: period_dates(
            period,
            fallback_year.unwrap_or_else(|| chrono::Local::now().year()),
        ),
        ..Digest::default()
    };
    let mut front = BTreeMap::<&str, Vec<&str>>::new();
    let mut current_front = None;
    let mut summary_lines: Vec<String> = Vec::new();
    let mut in_summary = false;
    while let Some(line) = lines.peek() {
        let s = line.trim();
        if ["## ", "### ", "#### "]
            .iter()
            .any(|prefix| s.starts_with(prefix))
        {
            break;
        }
        if current_front.is_none() {
            if let Some(quoted) = s.strip_prefix('>') {
                let q = quoted.trim();
                if let Some(rest) = strip_summary_prefix(q) {
                    in_summary = true;
                    if !rest.is_empty() {
                        summary_lines.push(rest.to_owned());
                    }
                    lines.next();
                    continue;
                } else if in_summary && !q.is_empty() {
                    summary_lines.push(q.to_owned());
                    lines.next();
                    continue;
                }
            } else if !s.is_empty() {
                in_summary = false;
            }
        }
        if !matches!(s, "---" | "") {
            if let Some((_, key)) = i18n::FRONT_BLOCKS
                .iter()
                .find(|(label, _)| *label == s.trim_matches('*').trim())
            {
                in_summary = false;
                current_front = Some(*key);
            } else if let Some(key) = current_front {
                front.entry(key).or_default().push(line);
            }
        }
        lines.next();
    }
    digest.summary = summary_lines.join(" ").trim().to_owned();
    parse_toc(&front.remove("toc").unwrap_or_default(), &mut digest);
    let mut current = None;
    let mut body = Vec::new();
    let mut ids = HashSet::new();
    for raw in lines {
        let line = raw.trim();
        let item = ITEM.captures(line);
        if line == "---" || line.starts_with("## ") || line.starts_with("#### ") || item.is_some() {
            finish_item(&mut digest.sections, current.take(), &mut body);
        }
        if line == "---" {
            continue;
        }
        if let Some(title) = line.strip_prefix("## ") {
            push_section(&mut digest.sections, title.trim(), &mut ids);
        } else if let Some(title) = line.strip_prefix("#### ") {
            if digest.sections.is_empty() {
                push_section(
                    &mut digest.sections,
                    i18n::t(lang, "sec_default_papers"),
                    &mut ids,
                );
            }
            digest
                .sections
                .last_mut()
                .unwrap()
                .entries
                .push(SectionEntry::Subheading(title.trim().into()));
        } else if let Some(m) = item {
            if digest.sections.is_empty() {
                push_section(
                    &mut digest.sections,
                    i18n::t(lang, "sec_default_items"),
                    &mut ids,
                );
            }
            let title = m[3].trim();
            current = Some(Item {
                id: unique_id(item_anchor(&m[2]), &mut ids),
                number: m[2].into(),
                is_new: m.get(1).is_some() || title.starts_with("🆕"),
                title: title.strip_prefix("🆕").unwrap_or(title).trim().into(),
                ..Item::default()
            });
        } else if current.is_some() {
            body.push(raw);
        } else if !line.is_empty() {
            if let Some(sec) = digest.sections.last_mut() {
                if !sec.note.is_empty() {
                    sec.note.push('\n');
                }
                sec.note.push_str(line);
            }
        }
    }
    finish_item(&mut digest.sections, current, &mut body);
    let n = digest.items().count();
    ensure!(n > 0 && !digest.sections.is_empty(), "malformed digest markdown: parsed {} sections and {n} items (expected section headings and numbered items)", digest.sections.len());
    if digest.toc_groups.is_empty() {
        let entries = digest
            .items()
            .map(|it| TocEntry {
                number: it.number.clone(),
                title: it.title.clone(),
                is_new: it.is_new,
            })
            .collect();
        digest.toc_groups.push(TocGroup {
            entries,
            ..TocGroup::default()
        });
    }
    digest.must = split_list(&front.remove("must").unwrap_or_default())
        .into_iter()
        .map(|raw| {
            let (title, when, dek) = if let Some(m) = MUST_BOLD.captures(&raw) {
                (
                    m[1].trim().to_owned(),
                    m.get(2).map_or("", |s| s.as_str().trim()).to_owned(),
                    m[3].trim().to_owned(),
                )
            } else if let Some(m) = MUST_PLAIN.captures(&raw) {
                (
                    m[1].replace("**", "").trim().to_owned(),
                    String::new(),
                    m[2].trim().into(),
                )
            } else {
                (
                    raw.replace("**", "").trim().into(),
                    String::new(),
                    String::new(),
                )
            };
            let anchor = guess_anchor(&title, &digest);
            MustItem {
                title,
                when,
                dek,
                anchor,
            }
        })
        .collect();
    digest.themes = split_list(&front.remove("themes").unwrap_or_default())
        .into_iter()
        .map(|raw| {
            if let Some((title, body)) = raw
                .strip_prefix("**")
                .and_then(|rest| rest.split_once("**"))
            {
                ThemeItem {
                    title: title.trim().into(),
                    body: body.trim().into(),
                }
            } else {
                ThemeItem {
                    body: raw,
                    ..ThemeItem::default()
                }
            }
        })
        .collect();
    Ok(digest)
}

fn strip_summary_prefix(text: &str) -> Option<&str> {
    let prefixes = [
        "**本期摘要：**",
        "**本期摘要:**",
        "**摘要：**",
        "**摘要:**",
        "**Summary:**",
        "**Summary：**",
        "**summary:**",
        "本期摘要：",
        "本期摘要:",
        "摘要：",
        "摘要:",
        "Summary:",
        "Summary：",
    ];
    for p in prefixes {
        if let Some(rest) = text.strip_prefix(p) {
            return Some(rest.trim());
        }
    }
    if text.len() >= 8 && text[..8].eq_ignore_ascii_case("summary:") {
        return Some(text[8..].trim());
    }
    None
}

pub fn item_anchor(number: &str) -> String {
    let parts = number
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    format!("item-{}", parts.join("-"))
}

fn unique_id(base: String, ids: &mut HashSet<String>) -> String {
    if ids.insert(base.clone()) {
        return base;
    }
    for i in 2.. {
        let id = format!("{base}-{i}");
        if ids.insert(id.clone()) {
            return id;
        }
    }
    unreachable!()
}

fn push_section(sections: &mut Vec<Section>, title: &str, ids: &mut HashSet<String>) {
    let low = LEAD_EMOJI.replace(title, "").to_lowercase();
    let base = i18n::SECTION_IDS
        .iter()
        .find(|(words, _)| words.iter().any(|word| low.contains(word)))
        .map(|(_, id)| (*id).to_owned())
        .unwrap_or_else(|| format!("sec-{}", sections.len() + 1));
    sections.push(Section {
        id: unique_id(base, ids),
        title: title.into(),
        ..Section::default()
    });
}

pub fn period_dates(period: &str, fallback_year: i32) -> Vec<NaiveDate> {
    static SLASH: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?:([0-9]{4})[/-])?([0-9]{1,2})/([0-9]{1,2})").unwrap());
    static ISO: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"([0-9]{4})-([0-9]{1,2})-([0-9]{1,2})").unwrap());
    let matches: Vec<_> = if SLASH.is_match(period) {
        SLASH.captures_iter(period).collect()
    } else {
        ISO.captures_iter(period).collect()
    };
    let mut dates: Vec<NaiveDate> = Vec::new();
    for m in matches {
        let (Ok(month), Ok(day)) = (m[2].parse(), m[3].parse()) else {
            continue;
        };
        let mut year = m
            .get(1)
            .and_then(|v| v.as_str().parse().ok())
            .unwrap_or(fallback_year);
        if m.get(1).is_none() {
            if let Some(last) = dates.last() {
                year = last.year() + i32::from((month, day) < (last.month(), last.day()));
            }
        }
        dates.extend(NaiveDate::from_ymd_opt(year, month, day));
    }
    dates
}

fn parse_toc(lines: &[&str], digest: &mut Digest) {
    let mut notes = Vec::new();
    for line in lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
    {
        if line.starts_with(['(', '（']) {
            notes.push(line);
            continue;
        }
        let (label, rest) = line
            .strip_prefix("**")
            .and_then(|s| s.split_once("**"))
            .unwrap_or(("", line));
        let entries: Vec<_> = TOC_SPLIT
            .split(rest.trim())
            .filter_map(|s| TOC_ENTRY.captures(s.trim()))
            .map(|m| TocEntry {
                number: m[2].into(),
                title: m[3].replace("🆕", "").trim().into(),
                is_new: m.get(1).is_some() || m[3].contains("🆕"),
            })
            .collect();
        let note = if entries.is_empty() {
            rest.trim().into()
        } else {
            String::new()
        };
        if !label.is_empty() || !rest.trim().is_empty() {
            digest.toc_groups.push(TocGroup {
                label: label.trim().into(),
                entries,
                note,
            });
        }
    }
    digest.toc_note = notes.join("\n");
}

fn parse_markdown_links(line: &str) -> Vec<LinkItem> {
    let mut links = Vec::new();
    let mut active_url: Option<String> = None;
    let mut active_label = String::new();
    for event in MdParser::new_ext(line, MdOptions::empty()) {
        match event {
            Event::Start(Tag::Link { dest_url, .. }) => {
                active_url = Some(dest_url.into_string());
                active_label.clear();
            }
            Event::Text(text) | Event::Code(text) if active_url.is_some() => {
                active_label.push_str(&text);
            }
            Event::End(TagEnd::Link) => {
                if let Some(url) = active_url.take() {
                    let label = active_label.trim();
                    let url = url.trim();
                    if !label.is_empty() && !url.is_empty() {
                        links.push(LinkItem {
                            label: label.into(),
                            url: url.into(),
                        });
                    }
                }
                active_label.clear();
            }
            _ => {}
        }
    }
    links
}

fn finish_item(sections: &mut [Section], current: Option<Item>, body: &mut Vec<&str>) {
    let Some(mut it) = current else {
        return;
    };
    let lines = std::mem::take(body);
    let mut lines = lines
        .into_iter()
        .skip_while(|l| l.trim().is_empty())
        .peekable();
    if let Some(meta) = lines
        .peek()
        .map(|l| l.trim())
        .filter(|s| s.starts_with('`') && s.ends_with('`') && s.matches('`').count() == 2)
    {
        it.meta = meta.trim_matches('`').trim().into();
        let mut parts = it.meta.split('·').map(str::trim).filter(|s| !s.is_empty());
        it.source = parts.next().unwrap_or("").into();
        for part in parts {
            if SHORT_DATE.find(part).is_some_and(|m| m.start() == 0) {
                it.when = SHORT_DATE.replace_all(part, "$1.$2").into_owned();
            } else {
                for tag in part
                    .split(['/', '／', '、', '｜', '|'])
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    if !it.tags.iter().any(|existing| existing == tag) {
                        it.tags.push(tag.into());
                    }
                }
            }
        }
        lines.next();
    }
    let mut pending = "";
    let mut fields = BTreeMap::<&str, Vec<String>>::new();
    for line in lines {
        let s = line.trim();
        if s.starts_with("🔗") {
            it.links.extend(parse_markdown_links(s));
        } else if let Some(m) = FIELD.captures(s) {
            pending = i18n::LABEL_MAP
                .iter()
                .find(|(label, _)| *label == &m[1])
                .unwrap()
                .1;
            fields.entry(pending).or_default().push(m[2].into());
        } else {
            fields.entry(pending).or_default().push(if s.is_empty() {
                String::new()
            } else {
                line.into()
            });
        }
    }
    for (key, dst) in [
        ("", &mut it.extra),
        ("dek", &mut it.dek),
        ("look", &mut it.look),
        ("detail", &mut it.detail),
        ("followup", &mut it.followup),
    ] {
        *dst = fields
            .remove(key)
            .unwrap_or_default()
            .join("\n")
            .trim()
            .into();
    }
    sections
        .last_mut()
        .expect("item has a section")
        .entries
        .push(SectionEntry::Item(Box::new(it)));
}

fn split_list(lines: &[&str]) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = Vec::new();
    for s in lines.iter().map(|s| s.trim()) {
        if let Some(m) = LIST.find(s) {
            if !current.is_empty() {
                parts.push(current.join("\n").trim().into());
                current.clear();
            }
            current.push(&s[m.end()..]);
        } else if !s.is_empty() || !current.is_empty() {
            current.push(s);
        }
    }
    if !current.is_empty() {
        parts.push(current.join("\n").trim().into());
    }
    parts
}

fn guess_anchor(title: &str, digest: &Digest) -> String {
    let plain = PARENS.replace_all(title, "");
    let key = ANCHOR_SPLIT.split(&plain).next().unwrap_or("").trim();
    if key.is_empty() {
        return String::new();
    }
    digest
        .items()
        .find(|it| it.title.contains(key) || title.contains(&it.title) || key.contains(&it.title))
        .or_else(|| {
            digest.items().find(|it| {
                ANCHOR_TOKEN
                    .find_iter(key)
                    .any(|token| it.title.contains(token.as_str()))
            })
        })
        .map_or_else(String::new, |it| format!("#{}", it.id))
}

#[derive(Default)]
struct Node {
    name: String,
    attrs: BTreeMap<String, String>,
    children: Vec<usize>,
    text: String,
}

impl Node {
    fn attr(&self, key: &str) -> &str {
        self.attrs.get(key).map_or("", String::as_str)
    }
    fn class(&self, class: &str) -> bool {
        self.attr("class")
            .split_ascii_whitespace()
            .any(|s| s == class)
    }
    fn secondary(&self) -> bool {
        self.class("lang-pane")
            && (self.class("lang-en") || self.attr("data-lang").eq_ignore_ascii_case("en"))
    }
}

const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

struct Tree(Vec<Node>);

impl Tree {
    fn parse(html: &str) -> Self {
        let mut nodes = vec![Node::default()];
        let mut stack = vec![0usize];
        for token in html_tokens(html) {
            let parent = *stack.last().unwrap();
            let node = match token {
                Token::StartTag(tag) => Node {
                    name: String::from_utf8_lossy(&tag.name.0).into_owned(),
                    attrs: tag
                        .attributes
                        .into_iter()
                        .map(|(k, v)| {
                            (
                                String::from_utf8_lossy(&k.0).into_owned(),
                                String::from_utf8_lossy(&v.0).into_owned(),
                            )
                        })
                        .collect(),
                    ..Node::default()
                },
                Token::String(s) => Node {
                    text: String::from_utf8_lossy(&s.0).into_owned(),
                    ..Node::default()
                },
                Token::EndTag(tag) => {
                    let name = String::from_utf8_lossy(&tag.name.0);
                    if let Some(i) = stack.iter().rposition(|&i| nodes[i].name == name) {
                        stack.truncate(i);
                    }
                    continue;
                }
                _ => continue,
            };
            let idx = nodes.len();
            let svg_void = matches!(node.name.as_str(), "path" | "use" | "circle" | "rect")
                && stack.iter().any(|&i| nodes[i].name == "svg");
            if !node.name.is_empty() && !VOID.contains(&node.name.as_str()) && !svg_void {
                stack.push(idx);
            }
            nodes[parent].children.push(idx);
            nodes.push(node);
        }
        Self(nodes)
    }

    fn descendants(&self, root: usize) -> impl Iterator<Item = usize> + '_ {
        let mut stack = self.0[root]
            .children
            .iter()
            .rev()
            .copied()
            .collect::<Vec<_>>();
        std::iter::from_fn(move || {
            while let Some(i) = stack.pop() {
                let node = &self.0[i];
                if node.secondary()
                    || matches!(
                        node.name.as_str(),
                        "style" | "script" | "svg" | "button" | "template"
                    )
                {
                    continue;
                }
                stack.extend(node.children.iter().rev().copied());
                return Some(i);
            }
            None
        })
    }

    fn first(&self, root: usize, pred: impl Fn(&Node) -> bool) -> Option<usize> {
        self.descendants(root).find(|&i| pred(&self.0[i]))
    }

    fn text_of(&self, root: usize, omit: &[&str], paragraphs: bool) -> String {
        let mut out = String::new();
        let mut stack = vec![root];
        while let Some(i) = stack.pop() {
            let n = &self.0[i];
            if n.secondary()
                || matches!(
                    n.name.as_str(),
                    "script" | "style" | "svg" | "button" | "template"
                )
                || omit.iter().any(|class| n.class(class))
            {
                continue;
            }
            if n.class("math") && !n.attr("aria-label").is_empty() {
                out.push('$');
                out.push_str(n.attr("aria-label"));
                out.push('$');
                continue;
            }
            if paragraphs && (matches!(n.name.as_str(), "p" | "li" | "pre") || n.name == "br") {
                out.push_str("\n\n");
            }
            out.push_str(&n.text);
            stack.extend(n.children.iter().rev().copied());
        }
        if paragraphs {
            out.split("\n\n")
                .map(clean_text)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n")
        } else {
            clean_text(&out)
        }
    }

    fn text(&self, root: usize) -> String {
        self.text_of(root, &[], false)
    }
    fn first_class_text(&self, root: usize, class: &str) -> String {
        self.first(root, |n| n.class(class))
            .map_or_else(String::new, |i| self.text(i))
    }
}

pub fn parse_digest_html(html: &str) -> Result<ParsedIssue> {
    parse_digest_html_with_lang(html, None)
}

pub fn parse_digest_html_with_lang(html: &str, lang: Option<Lang>) -> Result<ParsedIssue> {
    let tree = Tree::parse(html);
    let lang = lang.unwrap_or_else(|| {
        tree.first(0, |n| n.name == "html")
            .and_then(|i| i18n::normalize_lang(tree.0[i].attr("lang")).ok())
            .unwrap_or_default()
    });
    let title = tree
        .descendants(0)
        .filter_map(|i| {
            let n = &tree.0[i];
            (n.name == "meta" && n.attr("name").eq_ignore_ascii_case("digest-title"))
                .then(|| clean_text(n.attr("content")))
        })
        .filter(|s| !s.is_empty())
        .last()
        .or_else(|| tree.first(0, |n| n.name == "title").map(|i| tree.text(i)))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "AI Digest".into());
    let mut issue = ParsedIssue {
        lang,
        title,
        edition: tree.first_class_text(0, "edition"),
        ..ParsedIssue::default()
    };
    if let Some(i) = tree.first(0, |n| n.class("mast-meta")) {
        for &child in &tree.0[i].children {
            let n = &tree.0[child];
            if n.name != "span" {
                continue;
            }
            let text = tree.text(child);
            if n.class("motto") {
                issue.motto = text;
            } else if !text.is_empty() {
                issue.meta_spans.push(text);
            }
        }
        issue.period_line = issue.meta_spans.first().cloned().unwrap_or_default();
        issue.stats_line = issue.meta_spans.get(1).cloned().unwrap_or_default();
    }
    if let Some(list) = tree.first(0, |n| n.name == "ol" && n.class("must-list")) {
        for i in tree.0[list]
            .children
            .iter()
            .copied()
            .filter(|&i| tree.0[i].name == "li")
        {
            let Some(title_node) = tree.first(i, |n| n.class("must-title")) else {
                continue;
            };
            let title = tree.text(title_node);
            if title.is_empty() {
                continue;
            }
            let n = &tree.0[title_node];
            let anchor = if n.name == "a" {
                n.attr("href").into()
            } else {
                String::new()
            };
            issue.must.push(ParsedMust {
                title,
                dek: tree.first_class_text(i, "must-dek"),
                anchor,
            });
        }
    }
    let opus = tree
        .first(0, |n| n.name == "header" && n.class("folio"))
        .is_some();
    if opus {
        for i in tree
            .descendants(0)
            .filter(|&i| tree.0[i].class("theme-item"))
        {
            let title = tree
                .first(i, |n| n.name == "h4")
                .map_or_else(String::new, |node| tree.text(node));
            let body = tree.text_of(i, &["theme-title"], true);
            issue.themes.push(ThemeItem { title, body });
        }
    }
    let mut story_sections = BTreeMap::new();
    for i in tree
        .descendants(0)
        .filter(|&i| tree.0[i].name == "section" && tree.0[i].class("sec"))
    {
        let title = tree
            .first(i, |n| n.name == "h2")
            .map_or_else(String::new, |h| tree.text_of(h, &["sec-no"], false));
        for story in tree.descendants(i).filter(|&j| is_story(&tree.0[j])) {
            story_sections.insert(story, issue.sections.len());
        }
        issue.sections.push(ParsedSection {
            id: tree.0[i].attr("id").into(),
            title,
            items: Vec::new(),
        });
    }
    let mut counts = BTreeMap::<String, usize>::new();
    for i in tree.descendants(0).filter(|&i| is_story(&tree.0[i])) {
        let section = story_sections.get(&i).copied();
        let item = parse_story(
            &tree,
            i,
            section.map_or("", |idx| &issue.sections[idx].title),
            opus,
        );
        if item.title.is_empty() {
            continue;
        }
        for tag in &item.tags {
            *counts.entry(tag.clone()).or_default() += 1;
        }
        if let Some(idx) = section {
            issue.sections[idx].items.push(item.clone());
        }
        issue.items.push(item);
    }
    let mut counts: Vec<_> = counts.into_iter().collect();
    counts.sort_by(|(a, ac), (b, bc)| bc.cmp(ac).then_with(|| a.cmp(b)));
    issue.top_tags = counts.into_iter().take(10).map(|(tag, _)| tag).collect();
    issue.items_count = issue.items.len();
    issue.description = issue_description(&issue);
    Ok(issue)
}

fn is_story(n: &Node) -> bool {
    n.name == "article" && n.class("story")
}

fn parse_story(tree: &Tree, i: usize, section: &str, opus: bool) -> ParsedItem {
    static TITLE_NO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\d+\.\s*").unwrap());
    let title = tree
        .first(i, |n| n.name == "h3")
        .map(|h| {
            TITLE_NO
                .replace(&tree.text_of(h, &["story-no"], false), "")
                .trim()
                .to_owned()
        })
        .unwrap_or_default();
    let field = |key, label| {
        tree.descendants(i)
            .filter(|&j| tree.0[j].class(key))
            .map(|j| tree.text_of(j, &[label], opus))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(if opus { "\n\n" } else { " " })
    };
    let mut links = Vec::new();
    for root in tree
        .descendants(i)
        .filter(|&j| tree.0[j].class("story-links"))
    {
        for a in tree.descendants(root).filter(|&j| tree.0[j].name == "a") {
            let label = tree.text(a);
            let url = tree.0[a].attr("href");
            if !label.is_empty() && !url.is_empty() {
                links.push((label, url.into()));
            }
        }
    }
    ParsedItem {
        id: tree.0[i].attr("id").into(),
        number: tree
            .first_class_text(i, "story-no")
            .trim_end_matches('.')
            .into(),
        title,
        source: tree.first_class_text(i, "source"),
        when: tree.first_class_text(i, "when"),
        tags: tree.0[i]
            .attr("data-tags")
            .split('|')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        dek: field("dek", ""),
        look: field("look", "look-k"),
        detail: field("detail", "detail-k"),
        extra: field("extra", "extra-k"),
        links,
        section: section.into(),
    }
}

fn issue_description(issue: &ParsedIssue) -> String {
    let ParsedIssue {
        title,
        period_line,
        items_count,
        lang,
        ..
    } = issue;
    let leads = issue
        .items
        .iter()
        .take(3)
        .map(|it| it.title.as_str())
        .collect::<Vec<_>>();
    if leads.is_empty() {
        return truncate(title, 160);
    }
    let prefix = match (*lang, period_line.is_empty()) {
        (Lang::En, true) => format!("{title}: "),
        (Lang::En, false) => format!("{title} ({period_line}, {items_count} items): "),
        (Lang::ZhCn, true) => format!("{title}："),
        (Lang::ZhCn, false) => format!("{title}（{period_line}，共 {items_count} 条）："),
    };
    truncate(&(prefix + &leads.join(lang.choose("; ", "；"))), 160)
}

fn truncate(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s.into();
    }
    let cut: String = s.chars().take(limit - 1).collect();
    let cut = cut.trim_end_matches([' ', ',', ';', '·', '|', '—', '-']);
    format!("{cut}…")
}
