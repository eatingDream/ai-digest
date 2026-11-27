use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    fmt::Write,
    ops::Range,
    sync::LazyLock,
};

use chrono::{Datelike, NaiveDate};
use pulldown_cmark::{html, Alignment, CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};
use regex::Regex;

use crate::{
    assets,
    i18n::{self, t, Lang},
    math::{math_ranges, render_inline_math},
    model::{
        canon_url, full_width_len, html_escape as esc, normalized_safe_url, safe_external_url,
        safe_internal_url, strip_marked, strip_tracking_params, Digest, Item, LangAlt, LinkItem,
        SectionEntry,
    },
};

const TOP_SLOT: &str = "<!-- digest-nav-slot -->";
const BOTTOM_SLOT: &str = "<!-- digest-nav-bottom-slot -->";

// Native pane selection also works before the reader script initializes.
const BILINGUAL_CSS: &str = ".lang-en,#en-language:checked~.lang-zh,#language-zh:not(:checked)~.lang-zh:has(~.lang-en :target){display:none}#en-language:checked~.lang-en,#language-zh:not(:checked)~.lang-en:has(:target){display:block}.lang-seg label{cursor:pointer}.sr-only:focus-visible~.lang-pane .lang-seg{outline:2px solid var(--accent)}";

#[derive(Clone, Debug)]
pub struct RenderOptions {
    pub home_href: String,
    pub lang_alt: Option<LangAlt>,
    pub folio_dek: String,
    pub edition_number: Option<usize>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            home_href: "./".into(),
            lang_alt: None,
            folio_dek: String::new(),
            edition_number: None,
        }
    }
}

pub fn protect_technical_hyphens(text: &str) -> Cow<'_, str> {
    if !text.contains('-') {
        return Cow::Borrowed(text);
    }
    let bytes = text.as_bytes();
    let mut out: Option<String> = None;
    let mut copied = 0;
    for i in 1..bytes.len().saturating_sub(1) {
        if bytes[i] == b'-'
            && bytes[i - 1].is_ascii_alphanumeric()
            && bytes[i + 1].is_ascii_alphanumeric()
        {
            let buf = out.get_or_insert_with(|| String::with_capacity(text.len() + 8));
            buf.push_str(&text[copied..i]);
            buf.push('\u{2011}');
            copied = i + 1;
        }
    }
    match out {
        Some(mut buf) => {
            buf.push_str(&text[copied..]);
            Cow::Owned(buf)
        }
        None => Cow::Borrowed(text),
    }
}

pub fn render_markdown(text: &str, inline: bool) -> String {
    markdown(text, inline, "")
}

fn markdown(text: &str, inline: bool, prefix: &str) -> String {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut slot = "\u{e000}formula";
    let mut alternate_slot = String::new();
    if text.contains(slot) {
        alternate_slot.push_str(slot);
        while text.contains(&alternate_slot) {
            alternate_slot.push('\u{e000}');
        }
        slot = &alternate_slot;
    }
    let (prepared, formulas) = protect_math(text, options, slot);
    let mut events = Vec::new();
    let mut links = Vec::new();
    let mut autolinks = Vec::new();
    let mut in_code_block = false;
    let mut table_aligns: Vec<Vec<Alignment>> = Vec::new();
    let mut in_table_head = false;
    let mut cell_idx = 0usize;
    let mut skip = 0usize;
    for event in Parser::new_ext(&prepared, options) {
        if skip != 0 {
            match event {
                Event::Start(Tag::Image { .. } | Tag::Link { .. }) => skip += 1,
                Event::End(TagEnd::Image | TagEnd::Link) => skip -= 1,
                _ => {}
            }
            continue;
        }
        match event {
            Event::Start(
                Tag::Image { .. }
                | Tag::Link {
                    link_type: LinkType::Email,
                    ..
                },
            ) => skip += 1,
            Event::Html(_) | Event::InlineHtml(_) => {}
            Event::Start(Tag::CodeBlock(_)) if inline => {
                in_code_block = true;
            }
            Event::End(TagEnd::CodeBlock) if inline => {
                in_code_block = false;
                events.push(Event::Text(CowStr::Borrowed(" ")));
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                in_code_block = true;
                events.push(Event::Start(Tag::CodeBlock(kind)));
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                events.push(Event::End(TagEnd::CodeBlock));
            }
            Event::Start(
                Tag::Paragraph
                | Tag::Heading { .. }
                | Tag::BlockQuote(_)
                | Tag::List(_)
                | Tag::Item
                | Tag::Table(_)
                | Tag::TableHead
                | Tag::TableRow
                | Tag::TableCell,
            ) if inline => {}
            Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::BlockQuote(_)
                | TagEnd::List(_)
                | TagEnd::Item
                | TagEnd::Table
                | TagEnd::TableHead
                | TagEnd::TableRow
                | TagEnd::TableCell,
            ) if inline => events.push(Event::Text(CowStr::Borrowed(" "))),
            Event::Start(Tag::Table(aligns)) => {
                table_aligns.push(aligns);
                events.push(Event::Start(Tag::Table(Vec::new())));
            }
            Event::End(TagEnd::Table) => {
                table_aligns.pop();
                events.push(Event::End(TagEnd::Table));
            }
            Event::Start(Tag::TableHead) => {
                in_table_head = true;
                cell_idx = 0;
                events.push(Event::Start(Tag::TableHead));
            }
            Event::End(TagEnd::TableHead) => {
                in_table_head = false;
                events.push(Event::End(TagEnd::TableHead));
            }
            Event::Start(Tag::TableRow) => {
                cell_idx = 0;
                events.push(Event::Start(Tag::TableRow));
            }
            Event::Start(Tag::TableCell) => {
                let tag = if in_table_head { "th" } else { "td" };
                let cls = table_aligns
                    .last()
                    .and_then(|row| row.get(cell_idx))
                    .and_then(|align| match align {
                        Alignment::Left => Some(r#" class="ta-l""#),
                        Alignment::Center => Some(r#" class="ta-c""#),
                        Alignment::Right => Some(r#" class="ta-r""#),
                        Alignment::None => None,
                    })
                    .unwrap_or("");
                cell_idx += 1;
                events.push(Event::InlineHtml(CowStr::from(format!("<{tag}{cls}>"))));
            }
            Event::End(TagEnd::TableCell) => {
                let tag = if in_table_head { "th" } else { "td" };
                events.push(Event::InlineHtml(CowStr::from(format!("</{tag}>"))));
            }
            Event::Rule if inline => events.push(Event::Text(CowStr::Borrowed(" "))),
            Event::TaskListMarker(checked) => {
                events.push(Event::Text(CowStr::Borrowed(if checked {
                    "☑ "
                } else {
                    "☐ "
                })));
            }
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                ..
            }) => {
                let url = link_markup(&dest_url, &title, prefix, false);
                links.push(url.is_some());
                autolinks.push(matches!(link_type, LinkType::Autolink));
                if let Some(markup) = url {
                    events.push(Event::InlineHtml(CowStr::from(markup)));
                }
            }
            Event::End(TagEnd::Link) => {
                autolinks.pop();
                if links.pop().unwrap_or(false) {
                    events.push(Event::InlineHtml(CowStr::Borrowed("</a>")));
                }
            }
            Event::Text(s) if !formulas.is_empty() && s.contains(slot) => {
                let protect = !in_code_block && !autolinks.last().copied().unwrap_or(false);
                let mut rest = s.as_ref();
                while let Some((before, tail)) = rest.split_once(slot) {
                    let Some((key, after)) = tail.split_once('\u{e001}') else {
                        break;
                    };
                    let Some(html) = key.parse::<usize>().ok().and_then(|idx| formulas.get(idx))
                    else {
                        break;
                    };
                    let before_txt = if protect {
                        protect_technical_hyphens(before).into_owned()
                    } else {
                        before.to_owned()
                    };
                    events.push(Event::Text(CowStr::from(before_txt)));
                    events.push(Event::InlineHtml(CowStr::from(html.as_str())));
                    rest = after;
                }
                let rest_txt = if protect {
                    protect_technical_hyphens(rest).into_owned()
                } else {
                    rest.to_owned()
                };
                events.push(Event::Text(CowStr::from(rest_txt)));
            }
            Event::Text(s) if !in_code_block && !autolinks.last().copied().unwrap_or(false) => {
                match protect_technical_hyphens(&s) {
                    Cow::Borrowed(_) => events.push(Event::Text(s)),
                    Cow::Owned(protected) => events.push(Event::Text(CowStr::from(protected))),
                }
            }
            other => events.push(other),
        }
    }
    let mut rendered = String::new();
    html::push_html(&mut rendered, events.into_iter());
    rendered.trim().into()
}

fn protect_math<'a>(text: &'a str, options: Options, slot: &str) -> (Cow<'a, str>, Vec<String>) {
    let candidates = math_ranges(text);
    if candidates.is_empty() {
        return (Cow::Borrowed(text), Vec::new());
    }
    let mut text_ranges = Vec::new();
    let mut blocked = Vec::new();
    let mut link_ranges = Vec::new();
    let mut inline_html = Vec::new();
    for (event, range) in Parser::new_ext(text, options | Options::ENABLE_MATH).into_offset_iter() {
        match event {
            Event::Text(_) | Event::InlineMath(_) => text_ranges.push(range),
            Event::InlineHtml(_) => inline_html.push(range),
            Event::Code(_)
            | Event::Html(_)
            | Event::DisplayMath(_)
            | Event::Start(
                Tag::CodeBlock(_)
                | Tag::Image { .. }
                | Tag::Link {
                    link_type: LinkType::Autolink | LinkType::Email,
                    ..
                },
            ) => blocked.push(range),
            Event::Start(Tag::Link { .. }) => link_ranges.push(range),
            _ => {}
        }
    }
    let overlaps = |a: &Range<usize>, b: &Range<usize>| a.start < b.end && b.start < a.end;
    let mut formulas = Vec::new();
    let mut out = String::new();
    let mut copied = 0;
    for (range, _) in candidates {
        let open = range.start + usize::from(text[range.clone()].starts_with('\\'));
        let close = range.end - 1;
        if !text_ranges.iter().any(|r| r.contains(&open))
            || !text_ranges.iter().any(|r| r.contains(&close))
            || blocked.iter().any(|r| overlaps(r, &range))
            || inline_html
                .iter()
                .any(|r| overlaps(r, &range) && !(range.start < r.start && r.end < range.end))
            || link_ranges
                .iter()
                .any(|r| overlaps(r, &range) && !(r.start <= range.start && r.end >= range.end))
        {
            continue;
        }
        out.push_str(&text[copied..range.start]);
        write!(out, "{slot}{}\u{e001}", formulas.len()).unwrap();
        formulas.push(render_inline_math(&text[range.clone()]));
        copied = range.end;
    }
    if formulas.is_empty() {
        return (Cow::Borrowed(text), formulas);
    }
    out.push_str(&text[copied..]);
    (Cow::Owned(out), formulas)
}

fn link_markup(url: &str, title: &str, prefix: &str, all_external: bool) -> Option<String> {
    let external = all_external
        || url.trim().to_ascii_lowercase().starts_with("http://")
        || url.trim().to_ascii_lowercase().starts_with("https://");
    let url = if external {
        safe_external_url(url)?
    } else {
        safe_internal_url(url)?
    };
    let url = prefix_fragment(url, prefix);
    let mut out = format!(r#"<a href="{}""#, esc(&url));
    if !title.is_empty() {
        write!(out, r#" title="{}""#, esc(title)).unwrap();
    }
    if external {
        out.push_str(r#" target="_blank" rel="noopener noreferrer nofollow ugc""#);
    }
    out.push('>');
    Some(out)
}

pub fn compact_tab_title(digest: &Digest) -> String {
    if digest.weekly {
        digest
            .dates
            .last()
            .map(|d| format!("AI Digest · W{:02}", d.iso_week().week()))
            .unwrap_or_else(|| "AI Digest · Weekly".into())
    } else {
        digest
            .dates
            .last()
            .map(|d| format!("AI Digest · D{:02}/{:02}", d.month(), d.day()))
            .unwrap_or_else(|| "AI Digest".into())
    }
}

fn prefix_fragment(url: String, prefix: &str) -> String {
    match url.strip_prefix('#').filter(|anchor| !anchor.is_empty()) {
        Some(anchor) if !prefix.is_empty() => format!("#{prefix}{anchor}"),
        _ => url,
    }
}

pub(crate) const GEO_SLOT: &str = "<!-- digest-geo-head:start --><!-- digest-geo-head:end -->";

pub use crate::model::SPECULATION_RULES_JSON;

fn page_head(
    lang: Lang,
    title: &str,
    digest_title: Option<&str>,
    css: &str,
    noindex: bool,
) -> String {
    let mut out = format!(
        r##"<!doctype html>
<html lang="{lang}"><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<meta name="theme-color" content="#121110">
<meta name="color-scheme" content="dark">
<script>{}</script>
<title>{}</title>
"##,
        assets::BOOT_JS,
        esc(title)
    );
    if let Some(title) = digest_title {
        writeln!(
            out,
            r#"<meta name="digest-title" content="{}">"#,
            esc(title)
        )
        .unwrap();
    }
    out.push_str("<link rel=\"icon\" href=\"/favicon.svg\" type=\"image/svg+xml\">\n");
    if noindex {
        out.push_str("<meta name=\"robots\" content=\"noindex,nofollow\">\n");
    }
    write!(out, "{GEO_SLOT}\n<style>{css}</style>\n").unwrap();
    if !noindex {
        writeln!(
            out,
            "<script type=\"speculationrules\">{SPECULATION_RULES_JSON}</script>"
        )
        .unwrap();
    }
    out.push_str("</head>\n<body>\n");
    out
}

fn icon(name: &str) -> &'static str {
    match name {
        "search" => {
            r#"<svg aria-hidden="true" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><circle cx="7" cy="7" r="4.5"/><path d="m10.5 10.5 3 3"/></svg>"#
        }
        "theme" => {
            r#"<svg aria-hidden="true" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><circle cx="8" cy="8" r="5.5"/><path d="M8 2.5v11"/></svg>"#
        }
        "link" => {
            r#"<svg aria-hidden="true" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="m6 10 4-4m-5 2-2 2a2 2 0 0 0 3 3l2-2m0-6 2-2a2 2 0 0 1 3 3l-2 2"/></svg>"#
        }
        "close" => {
            r#"<svg aria-hidden="true" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="m4 4 8 8M4 12l8-8"/></svg>"#
        }
        "up" => {
            r#"<svg aria-hidden="true" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M8 13V3M4 7l4-4 4 4"/></svg>"#
        }
        "contents" => {
            r#"<svg aria-hidden="true" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M2 4h12M2 8h12M2 12h12"/></svg>"#
        }
        "ext" => {
            r#"<svg aria-hidden="true" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M5 11L11 5M6 5h5v5"/></svg>"#
        }
        _ => "",
    }
}

pub fn strip_leading_emoji(title: &str) -> &str {
    let trimmed = title.trim_start();
    let mut rest = trimmed;
    while let Some(ch) = rest.chars().next() {
        let is_emoji_or_mod = matches!(
            ch as u32,
            0x200D
                | 0x20E3
                | 0x23E9..=0x23FA
                | 0x2600..=0x27BF
                | 0x2B50
                | 0x2B55
                | 0xFE0E..=0xFE0F
                | 0x1F300..=0x1FAFF
        );
        if is_emoji_or_mod || ch.is_whitespace() {
            rest = &rest[ch.len_utf8()..];
        } else {
            break;
        }
    }
    if rest.is_empty() {
        trimmed
    } else {
        rest
    }
}

fn skip_link(out: &mut String, lang: Lang, prefix: &str) {
    writeln!(
        out,
        r##"<a class="skip-link" href="#{prefix}main">{}</a>"##,
        t(lang, "skip")
    )
    .unwrap();
}

fn lang_control(lang: Lang, alt: Option<&LangAlt>, bilingual: bool) -> String {
    let active = lang.choose("EN", "中");
    if bilingual {
        return format!(
            r#"<div class="lang-seg" aria-label="{}"><span aria-current="page">{active}</span><label class="ctrl-btn lang-btn" for="{}">{}</label></div>"#,
            esc(t(lang, "aria_lang_switch")),
            lang.choose("language-zh", "en-language"),
            lang.choose("中", "EN")
        );
    }
    if let Some((alt, href)) =
        alt.and_then(|alt| normalized_safe_url(&alt.base).map(|href| (alt, href)))
    {
        return format!(
            r#"<div class="lang-seg"><span aria-current="page">{active}</span><a class="ctrl-btn lang-btn" data-lang-link href="{}" hreflang="{}" aria-label="{}">{}</a></div>"#,
            esc(&href),
            alt.lang,
            esc(t(lang, "lang_toggle_title")),
            alt.lang.choose("EN", "中")
        );
    }
    String::new()
}

fn tags_by_count<'a>(tags: impl Iterator<Item = &'a String>) -> Vec<(&'a str, usize)> {
    let mut counts = HashMap::new();
    for tag in tags {
        *counts.entry(tag.as_str()).or_insert(0) += 1;
    }
    let mut counts: Vec<_> = counts.into_iter().collect();
    counts.sort_unstable_by(|(a, ac), (b, bc)| bc.cmp(ac).then_with(|| a.cmp(b)));
    counts
}

fn filter_panel(
    out: &mut String,
    lang: Lang,
    prefix: &str,
    total: usize,
    tags: &[(&str, usize)],
    archive: bool,
) {
    let placeholder = t(
        lang,
        if archive {
            "archive_filter_placeholder"
        } else {
            "filter_placeholder"
        },
    );
    let label = t(
        lang,
        if archive {
            "archive_filter_label"
        } else {
            "filter_label"
        },
    );
    writeln!(out, r#"<div class="search-scrim" data-search-scrim hidden></div>
<div class="filter-bar filter-drawer search-panel" id="{prefix}digestFilterBar" role="dialog" aria-modal="false" aria-label="{}" hidden aria-hidden="true" inert><div class="filter-inner"><div class="filter-input-wrap">{}<input type="search" class="filter-input" placeholder="{}" aria-label="{}" autocomplete="off"><span class="filter-count num" aria-live="polite" data-total="{total}">{total}</span><button type="button" class="filter-clear" hidden>{}</button><button type="button" class="filter-close" aria-label="{}">{}</button></div><div class="filter-tags">"#,
        esc(label), icon("search"), esc(placeholder), esc(label), t(lang, "filter_clear"), esc(t(lang, "search_close")), icon("close")).unwrap();
    for (tag, count) in tags {
        let tag = esc(tag);
        write!(out, r#"<button type="button" class="filter-tag" data-tag="{tag}" aria-pressed="false">{tag}<span class="tag-cnt num">{count}</span></button>"#).unwrap();
    }
    out.push_str("</div></div></div>\n");
}

fn filter_empty(out: &mut String, lang: Lang, archive: bool) {
    write!(out, r#"<div class="filter-empty" hidden><p>{}</p><button type="button" class="filter-clear">{}</button></div>"#,
        t(lang, if archive { "archive_filter_empty" } else { "filter_empty" }), t(lang, "filter_clear")).unwrap();
}

fn tools(out: &mut String, lang: Lang, prefix: &str, alt: Option<&LangAlt>, bilingual: bool) {
    write!(out, r#"<div class="mast-tools"><button type="button" class="ctrl-btn search-btn" data-filter-toggle aria-expanded="false" aria-controls="{prefix}digestFilterBar" aria-label="{}">{}<span>{}</span><kbd>/</kbd></button>{}<button type="button" class="ctrl-btn theme-btn" data-theme-toggle aria-label="{}">{}</button></div>"#,
        esc(t(lang, "search_toggle_title")), icon("search"), t(lang, "search_btn"), lang_control(lang, alt, bilingual),
        esc(t(lang, "theme_toggle_title")), icon("theme")).unwrap();
}

fn short_section_label(title: &str) -> String {
    let clean = strip_leading_emoji(title).trim();
    let trimmed = clean.split(['（', '(']).next().unwrap_or(clean).trim();
    if trimmed.is_empty() {
        clean.to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn render_cat_chips(out: &mut String, digest: &Digest, lang: Lang) {
    let total = digest.items().count();
    write!(
        out,
        r#"<button type="button" class="chip" data-cat="all" aria-pressed="true">{} <b class="n num">{total}</b></button>"#,
        t(lang, "filter_all")
    )
    .unwrap();
    for sec in &digest.sections {
        let count = sec.items().count();
        if count == 0 {
            continue;
        }
        let cat = esc(&sec.id);
        let label = esc(&protect_technical_hyphens(&short_section_label(&sec.title)));
        write!(
            out,
            r#"<button type="button" class="chip" data-cat="{cat}" aria-pressed="false">{label} <b class="n num">{count}</b></button>"#
        )
        .unwrap();
    }
}

fn item_word_units(item: &Item) -> usize {
    static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new("[A-Za-z0-9]+").unwrap());
    let (mut cjk, mut latin) = (0usize, 0usize);
    for text in [
        &item.title,
        &item.dek,
        &item.look,
        &item.detail,
        &item.followup,
        &item.extra,
    ] {
        cjk += text
            .chars()
            .filter(|ch| ('\u{4e00}'..='\u{9fff}').contains(ch))
            .count();
        latin += WORD.find_iter(text).count();
    }
    cjk + ((latin as f64) * 1.5).round() as usize
}

fn pick_highlights(digest: &Digest) -> Vec<(&crate::model::Section, &Item)> {
    let mut picked: Vec<(&crate::model::Section, &Item)> = Vec::new();
    let mut seen_ids = std::collections::HashSet::<&str>::new();
    let mut per_cat = HashMap::<&str, usize>::new();
    let target = digest.items().count().min(5);
    if target == 0 {
        return picked;
    }

    // 1. Explicit digest.must anchors first
    for must in &digest.must {
        if picked.len() >= target {
            break;
        }
        let Some(anchor) = must.anchor.strip_prefix('#') else {
            continue;
        };
        for sec in &digest.sections {
            if let Some(item) = sec.items().find(|it| it.id == anchor) {
                let cnt = per_cat.entry(sec.id.as_str()).or_insert(0);
                if *cnt < 2 && seen_ids.insert(item.id.as_str()) {
                    *cnt += 1;
                    picked.push((sec, item));
                }
                break;
            }
        }
    }
    // 2. Hot / new items next (max 2 per category)
    for sec in &digest.sections {
        for item in sec.items().filter(|it| it.is_new) {
            if picked.len() >= target {
                break;
            }
            let cnt = per_cat.entry(sec.id.as_str()).or_insert(0);
            if *cnt < 2 && seen_ids.insert(item.id.as_str()) {
                *cnt += 1;
                picked.push((sec, item));
            }
        }
    }
    // 3. Round-robin lead items per section until reaching target (max 2 per category)
    for pass in 0..2 {
        for sec in &digest.sections {
            if picked.len() >= target {
                break;
            }
            if let Some(item) = sec.items().nth(pass) {
                let cnt = per_cat.entry(sec.id.as_str()).or_insert(0);
                if *cnt < 2 && seen_ids.insert(item.id.as_str()) {
                    *cnt += 1;
                    picked.push((sec, item));
                }
            }
        }
    }
    picked
}

fn classify_source(url: &str, label: &str) -> (&'static str, u8, &'static str, String) {
    let parsed = url::Url::parse(url).ok();
    let host = parsed
        .as_ref()
        .and_then(|u| u.host_str())
        .unwrap_or("")
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let host_clean = host.strip_prefix("www.").unwrap_or(&host).to_owned();
    let path = parsed
        .as_ref()
        .map(|u| u.path().to_ascii_lowercase())
        .unwrap_or_default();
    let label_low = label.to_ascii_lowercase();

    let is_paper = matches!(
        host_clean.as_str(),
        "arxiv.org"
            | "openreview.net"
            | "aclanthology.org"
            | "proceedings.mlr.press"
            | "nature.com"
            | "science.org"
            | "biorxiv.org"
            | "medrxiv.org"
            | "alphaxiv.org"
    ) || (host_clean == "huggingface.co" && path.starts_with("/papers"));

    let is_code = matches!(
        host_clean.as_str(),
        "github.com" | "gitlab.com" | "codeberg.org"
    );

    let is_model = matches!(
        host_clean.as_str(),
        "huggingface.co" | "modelscope.cn" | "ollama.com"
    ) && !is_paper;

    let is_x = matches!(
        host_clean.as_str(),
        "x.com" | "twitter.com" | "mobile.twitter.com"
    );

    let is_wechat = host_clean == "mp.weixin.qq.com";

    let is_official_host = matches!(
        host_clean.as_str(),
        "openai.com"
            | "anthropic.com"
            | "deepmind.google"
            | "blog.google"
            | "research.google"
            | "ai.meta.com"
            | "deepseek.com"
            | "qwen.ai"
            | "qwenlm.github.io"
            | "mistral.ai"
            | "x.ai"
            | "nvidia.com"
            | "blogs.nvidia.com"
            | "developer.nvidia.com"
            | "microsoft.com"
            | "blogs.microsoft.com"
            | "apple.com"
            | "machinelearning.apple.com"
            | "moonshot.cn"
            | "zhipuai.cn"
            | "z.ai"
            | "minimax.io"
            | "bytedance.com"
            | "seed.bytedance.com"
            | "cohere.com"
            | "allenai.org"
            | "thinkingmachines.ai"
            | "cursor.com"
            | "vercel.com"
            | "cloudflare.com"
            | "blog.cloudflare.com"
    ) || label.contains("官方")
        || label.contains("官网")
        || label_low.contains("official")
        || label_low.contains("blog");

    let (kind, rank, key) = if is_paper {
        ("paper", 0, "src_paper")
    } else if is_official_host && !is_code && !is_model && !is_x && !is_wechat {
        ("official", 1, "src_official")
    } else if is_code {
        ("code", 2, "src_code")
    } else if is_model {
        ("model", 3, "src_model")
    } else if is_x {
        ("x", 4, "src_x")
    } else if is_wechat {
        ("wechat", 5, "src_wechat")
    } else {
        ("media", 6, "src_media")
    };
    let display = if host_clean.is_empty() {
        label.to_owned()
    } else {
        host_clean
    };
    (kind, rank, key, display)
}

struct Reader<'a> {
    digest: &'a Digest,
    lang: Lang,
    opts: &'a RenderOptions,
    prefix: &'a str,
    bilingual: bool,
}

impl Reader<'_> {
    fn render(&self, out: &mut String) {
        let Self {
            digest,
            lang,
            opts,
            prefix,
            bilingual,
        } = *self;
        let total = digest.items().count();
        let home = prefix_fragment(
            normalized_safe_url(&opts.home_href).unwrap_or_else(|| "./".into()),
            prefix,
        );
        let kind = if digest.weekly { "weekly" } else { "daily" };
        let edition = t(
            lang,
            if digest.weekly {
                "edition_weekly"
            } else {
                "edition_daily"
            },
        );
        let date = digest
            .dates
            .last()
            .map(|d| d.format(lang.choose("%b %-d", "%-m.%-d")).to_string())
            .unwrap_or_else(|| digest.period.clone());
        let date = if digest.weekly {
            esc(&digest.period)
        } else {
            esc(&date)
        };
        out.push_str("<div class=\"mast-sentinel\" aria-hidden=\"true\"></div>\n");
        writeln!(out, r#"<header class="mast" role="banner"><div class="mast-row"><span class="np"><a class="np-link" href="{}" data-brand-home><span class="ai">AI</span> Digest</a></span><span class="edition" data-kind="{kind}">{edition}{}{date}</span><span class="mast-sep" aria-hidden="true"></span>{TOP_SLOT}"#, esc(&home), if date.is_empty() { "" } else { " · " }).unwrap();
        tools(out, lang, prefix, opts.lang_alt.as_ref(), bilingual);
        out.push_str("</div></header>\n<div class=\"read-prog\" aria-hidden=\"true\"><div class=\"read-prog-fill\"></div></div>\n");
        filter_panel(
            out,
            lang,
            prefix,
            total,
            &tags_by_count(digest.items().flat_map(|it| &it.tags)),
            false,
        );
        out.push_str("<div class=\"reader-shell\">\n");
        self.toc(out);
        let period = match digest.dates.as_slice() {
            [a, .., b] if digest.weekly => i18n::date_range(lang, *a, *b),
            [.., d] if lang == Lang::En => {
                format!("{}, {}", i18n::weekday(lang, *d), i18n::date_long(lang, *d))
            }
            [.., d] => format!(
                "{} · {}",
                i18n::date_long(lang, *d),
                i18n::weekday(lang, *d)
            ),
            [] => digest.period.clone(),
        };
        let iso_date = digest
            .dates
            .last()
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        let heading_text = if period.is_empty() {
            esc(&protect_technical_hyphens(&digest.full_title()))
        } else if iso_date.is_empty() {
            esc(&period)
        } else {
            format!(
                r#"<time datetime="{}">{}</time>"#,
                esc(&iso_date),
                esc(&period)
            )
        };
        let edition_sr = if digest.weekly {
            lang.choose(" AI Digest Weekly", " AI Digest 周报")
        } else {
            lang.choose(" AI Digest Daily", " AI Digest 日报")
        };
        let mins = read_minutes(digest);
        let read_html = t(lang, "read_time").replace("{m}", &format!("<b>{mins}</b>"));
        let count_html = i18n::count_items(lang, total);
        writeln!(
            out,
            r#"<main id="{prefix}main" class="prose-col"><header class="folio issue-head"><h1 class="issue-title">{heading_text}<span class="vh">{edition_sr}</span></h1>"#
        )
        .unwrap();
        if !opts.folio_dek.is_empty() {
            writeln!(
                out,
                r#"<p class="folio-dek">{}</p>"#,
                markdown(&opts.folio_dek, true, prefix)
            )
            .unwrap();
        }
        write!(
            out,
            r#"<p class="issue-meta mast-meta"><span data-k="read" data-read-tpl="{}">{read_html}</span><span data-k="count" data-total="{total}">{count_html}</span><span class="meta-cats" data-meta-cats>"#,
            esc(t(lang, "read_time"))
        )
        .unwrap();
        for sec in &digest.sections {
            let count = sec.items().count();
            if count == 0 {
                continue;
            }
            write!(
                out,
                r#"<span class="meta-cat" data-cat="{}">{} <b class="num">{count}</b></span>"#,
                esc(&sec.id),
                esc(&protect_technical_hyphens(&short_section_label(&sec.title)))
            )
            .unwrap();
        }
        writeln!(
            out,
            r#"</span><button type="button" class="active-q-chip" data-active-q hidden></button></p>"#
        )
        .unwrap();
        writeln!(
            out,
            r#"<nav class="folio-sections" aria-label="{}">"#,
            esc(t(lang, "issue_sections"))
        )
        .unwrap();
        for (i, sec) in digest.sections.iter().enumerate() {
            let count = sec.items().count();
            if count == 0 {
                continue;
            }
            writeln!(out, r##"<a href="#{prefix}{}"><span class="num">{:02}</span> {} <span class="num">{count}</span></a>"##, esc(&sec.id), i + 1, esc(&protect_technical_hyphens(strip_leading_emoji(&sec.title)))).unwrap();
        }
        out.push_str("</nav></header>\n");
        write!(
            out,
            r#"<div class="mobile-filter-strip" role="group" aria-label="{}">"#,
            esc(t(lang, "filter_by_category"))
        )
        .unwrap();
        render_cat_chips(out, digest, lang);
        out.push_str("</div>\n");
        self.highlights(out);
        self.front(out);
        let mut lead = true;
        for (idx, section) in digest.sections.iter().enumerate() {
            let cat_id = esc(&section.id);
            writeln!(out, r#"<section class="sec" id="{prefix}{cat_id}" data-category="{cat_id}"><div class="sec-head"><span class="sec-no num">{:02}</span><h2>{}</h2><span class="sec-cnt num">{}</span>"#,
                idx + 1, esc(&protect_technical_hyphens(strip_leading_emoji(&section.title))), section.items().count()).unwrap();
            if !section.note.is_empty() {
                writeln!(
                    out,
                    r#"<p class="sec-note">{}</p>"#,
                    markdown(&section.note, true, prefix)
                )
                .unwrap();
            }
            out.push_str("</div>\n");
            for entry in &section.entries {
                match entry {
                    SectionEntry::Subheading(title) => writeln!(
                        out,
                        r#"<h3 class="subhead">{}</h3>"#,
                        esc(&protect_technical_hyphens(title))
                    )
                    .unwrap(),
                    SectionEntry::Item(item) => {
                        self.story(out, &section.id, item, lead);
                        lead = false;
                    }
                }
            }
            out.push_str("</section>\n");
        }
        filter_empty(out, lang, false);
        writeln!(out, r#"<div class="end"></div><footer class="colophon">{}</footer>{BOTTOM_SLOT}
</main></div>
<nav class="dock" aria-label="{}"><button type="button" class="dock-toc" data-sheet-open aria-controls="{prefix}toc-sheet">{}<span>{}</span></button><div class="dock-nav" data-dock-nav></div><button type="button" class="dock-top" data-scroll-top hidden aria-label="{}">{}</button></nav>
<button type="button" class="ctrl-btn to-top" data-to-top aria-label="{}" aria-hidden="true" inert>{}</button>
<div class="scrim" data-sheet-scrim hidden></div><div class="sr-only" id="{prefix}liveAnnounce" aria-live="polite"></div>"#,
            t(lang, "end"), esc(t(lang, "nav_aria")), icon("contents"), t(lang, "toc"), esc(t(lang, "to_top")), icon("up"), esc(t(lang, "to_top")), icon("up")).unwrap();
    }

    fn highlights(&self, out: &mut String) {
        let Self {
            digest,
            lang,
            prefix,
            ..
        } = *self;
        let items = pick_highlights(digest);
        if items.is_empty() {
            return;
        }
        let n = items.len();
        writeln!(
            out,
            r#"<section class="highlights" aria-labelledby="{prefix}hl-h"><h2 id="{prefix}hl-h">{} <span class="hl-count num">({n})</span></h2><ol class="hl-list">"#,
            t(lang, "highlights_title")
        )
        .unwrap();
        for (idx, (sec, item)) in items.iter().enumerate() {
            let extra = if idx >= 3 { " data-hl-extra" } else { "" };
            let cat_label = esc(&protect_technical_hyphens(&short_section_label(&sec.title)));
            let title = esc(&protect_technical_hyphens(&item.title));
            writeln!(
                out,
                r##"<li data-item-id="{prefix}{}" data-category="{}" data-tags="{}"{extra}><a href="#{prefix}{}"><span class="hl-cat">{cat_label}</span><span class="hl-t">{title}</span></a></li>"##,
                esc(&item.id),
                esc(&sec.id),
                esc(&item.tags.join("|")),
                esc(&item.id)
            )
            .unwrap();
        }
        writeln!(
            out,
            r#"</ol><p class="hl-empty" hidden>{} · <b class="num">0</b></p>{}</section>"#,
            t(lang, "highlights_empty"),
            if n > 3 {
                format!(
                    r#"<button type="button" class="hl-more" data-hl-more data-more-label="{}" data-less-label="{}">{}</button>"#,
                    esc(t(lang, "highlights_more")),
                    esc(t(lang, "highlights_less")),
                    t(lang, "highlights_more")
                )
            } else {
                String::new()
            }
        )
        .unwrap();
    }

    fn toc(&self, out: &mut String) {
        let Self {
            digest,
            lang,
            prefix,
            ..
        } = *self;
        write!(
            out,
            r#"<nav class="toc-side" id="{prefix}toc-sheet" aria-label="{}"><div class="toc-top"><div class="toc-handle" aria-hidden="true"></div><div class="toc-head"><h2 class="toc-label">{}</h2><button type="button" class="toc-expand-all" aria-expanded="true">{}</button><button type="button" class="toc-close" aria-label="{}">{}</button></div><div class="cat-chips" role="group" aria-label="{}">"#,
            esc(t(lang, "toc")),
            t(lang, "toc"),
            lang.choose("Collapse all", "全部收起"),
            esc(t(lang, "toc_close")),
            icon("close"),
            esc(t(lang, "filter_by_category"))
        )
        .unwrap();
        render_cat_chips(out, digest, lang);
        out.push_str("</div></div>\n");
        if !digest.must.is_empty() || !digest.themes.is_empty() {
            out.push_str("<div class=\"toc-front\">");
            if !digest.must.is_empty() {
                write!(
                    out,
                    r##"<a href="#{prefix}front-must">{}</a>"##,
                    t(lang, "must")
                )
                .unwrap();
            }
            if !digest.themes.is_empty() {
                write!(
                    out,
                    r##"<a href="#{prefix}front-themes">{}</a>"##,
                    t(lang, "themes")
                )
                .unwrap();
            }
            out.push_str("</div>\n");
        }
        for (idx, sec) in digest.sections.iter().enumerate() {
            let id = esc(&sec.id);
            let title = esc(&protect_technical_hyphens(strip_leading_emoji(&sec.title)));
            let count = sec.items().count();
            writeln!(out, r#"<div class="ts-group" data-sec="{prefix}{id}" data-category="{id}" data-open="true"><button type="button" class="ts-sec" aria-expanded="true" aria-controls="{prefix}ts-list-{id}"><span class="ts-no num">{:02}</span><span class="ts-title">{title}</span><span class="ts-cnt num" data-total="{count}">{count}</span></button><div class="ts-panel" id="{prefix}ts-list-{id}"><ol class="ts-items">"#,
                idx + 1).unwrap();
            for item in sec.items() {
                let raw_title = esc(&item.title);
                let title = esc(&protect_technical_hyphens(&item.title));
                let short_cls = if full_width_len(&item.title) <= 30.0 {
                    " short"
                } else {
                    ""
                };
                let num = esc(&item.number);
                writeln!(
                    out,
                    r##"<li data-item-id="{prefix}{}" data-category="{id}"><a href="#{prefix}{}" title="{raw_title}"><span class="ts-num num">{num}.</span><span class="ts-title{short_cls}">{title}</span>{}</a></li>"##,
                    esc(&item.id),
                    esc(&item.id),
                    if item.is_new {
                        r#"<span class="dot-hot" aria-hidden="true"></span>"#
                    } else {
                        ""
                    }
                )
                .unwrap();
            }
            out.push_str("</ol></div></div>\n");
        }
        out.push_str("</nav>\n");
    }

    fn front(&self, out: &mut String) {
        let Self {
            digest,
            lang,
            prefix,
            ..
        } = *self;
        if !digest.must.is_empty() {
            writeln!(out, r#"<section class="front front-must" id="{prefix}front-must"><h2 class="kicker">{}</h2><ol class="must-list">"#, t(lang, "must")).unwrap();
            for (i, must) in digest.must.iter().enumerate() {
                let when = if must.when.is_empty() {
                    String::new()
                } else {
                    match lang {
                        Lang::En => format!(" ({})", esc(&must.when)),
                        Lang::ZhCn => format!("（{}）", esc(&must.when)),
                    }
                };
                let title = esc(&protect_technical_hyphens(&must.title));
                write!(
                    out,
                    r#"<li><span class="must-n num">{:02}</span><div class="must-body">"#,
                    i + 1
                )
                .unwrap();
                if let Some(anchor) = must.anchor.strip_prefix('#').filter(|s| !s.is_empty()) {
                    write!(
                        out,
                        r##"<a class="must-title" href="#{prefix}{}">{title}{when}</a>"##,
                        esc(anchor)
                    )
                    .unwrap();
                } else {
                    write!(out, r#"<span class="must-title">{title}{when}</span>"#).unwrap();
                }
                if !must.dek.is_empty() {
                    write!(
                        out,
                        r#"<p class="must-dek">{}</p>"#,
                        markdown(&must.dek, true, prefix)
                    )
                    .unwrap();
                }
                out.push_str("</div></li>\n");
            }
            out.push_str("</ol></section>\n");
        }
        if !digest.themes.is_empty() {
            writeln!(out, r#"<section class="front front-lead" id="{prefix}front-themes"><h2 class="kicker">{}</h2><div class="themes">"#, t(lang, "themes")).unwrap();
            for theme in &digest.themes {
                let body = related_links(&theme.body, digest);
                write!(out, r#"<div class="theme-item">"#).unwrap();
                if !theme.title.is_empty() {
                    write!(
                        out,
                        r#"<h4 class="theme-title">{}</h4>"#,
                        markdown(&theme.title, true, prefix)
                    )
                    .unwrap();
                }
                writeln!(out, "{}</div>", markdown(&body, false, prefix)).unwrap();
            }
            out.push_str("</div></section>\n");
        }
    }

    fn story(&self, out: &mut String, category: &str, item: &Item, lead: bool) {
        let Self { lang, prefix, .. } = *self;
        let id = esc(&item.id);
        let cat = esc(category);
        let words = item_word_units(item);
        let lead_class = if lead { " story--lead" } else { "" };
        writeln!(
            out,
            r#"<article class="story{lead_class}" id="{prefix}{id}" data-item-id="{prefix}{id}" data-category="{cat}" data-tags="{}" data-words="{words}">"#,
            esc(&item.tags.join("|"))
        )
        .unwrap();
        out.push_str("<div class=\"story-meta\">");
        if !item.source.is_empty() {
            write!(out, "<span class=\"source\">{}</span>", esc(&item.source)).unwrap();
        }
        for tag in &item.tags {
            let accent = if i18n::BADGE_ACCENT_TAGS.contains(&tag.as_str()) {
                " badge-accent"
            } else {
                ""
            };
            let tag = esc(tag);
            write!(
                out,
                r#"<button type="button" class="badge{accent}" data-tag="{tag}">{tag}</button>"#
            )
            .unwrap();
        }
        if item.is_new {
            write!(
                out,
                r#"<span class="badge badge-hot">{}</span>"#,
                t(lang, "new")
            )
            .unwrap();
        }
        let hide_when = item.when.is_empty()
            || (!self.digest.weekly
                && self
                    .digest
                    .dates
                    .last()
                    .is_some_and(|d| same_short_date(&item.when, *d)));
        let when_hidden = if hide_when { " hidden" } else { "" };
        write!(out, r#"<time class="when"{when_hidden}>{}</time><button type="button" class="copy-link" data-copy-anchor="{prefix}{id}" aria-label="{}">{}</button></div>
<div class="story-head"><h3><span class="story-no">{}.</span> {}</h3></div>
"#, esc(&item.when), esc(t(lang, "copy_link")), icon("link"), esc(&item.number), markdown(&item.title, true, prefix)).unwrap();
        if !item.dek.is_empty() {
            let body = markdown(&item.dek, true, prefix);
            writeln!(out, r#"<p class="dek">{body}</p>"#).unwrap();
        }
        if !item.look.is_empty() {
            writeln!(
                out,
                r#"<aside class="look"><span class="look-k">{}</span><div class="prose">{}</div></aside>"#,
                t(lang, "label_look"),
                markdown(&item.look, false, prefix)
            )
            .unwrap();
        }
        for (class, label, value) in [
            ("detail", t(lang, "label_detail"), &item.detail),
            ("extra", t(lang, "label_followup"), &item.followup),
            ("extra", lang.choose("More", "补充"), &item.extra),
        ] {
            if !value.is_empty() {
                writeln!(out, r#"<div class="{class}"><span class="{class}-k">{label}</span><div class="prose">{}</div></div>"#,
                    markdown(value, false, prefix)).unwrap();
            }
        }
        if !item.links.is_empty() {
            struct RenderedSource<'a> {
                rank: u8,
                idx: usize,
                kind: &'static str,
                key: &'static str,
                host_display: String,
                safe_url: Option<String>,
                orig_label: &'a str,
            }
            let mut seen = std::collections::HashSet::<String>::new();
            let mut classified = Vec::new();
            for (idx, link) in item.links.iter().enumerate() {
                if let Some(safe) = safe_external_url(&link.url) {
                    let clean = strip_tracking_params(&safe);
                    if !seen.insert(canon_url(&clean)) {
                        continue;
                    }
                    let (kind, rank, key, host_display) = classify_source(&clean, &link.label);
                    classified.push(RenderedSource {
                        rank,
                        idx,
                        kind,
                        key,
                        host_display,
                        safe_url: Some(clean),
                        orig_label: link.label.as_str(),
                    });
                } else {
                    classified.push(RenderedSource {
                        rank: 99,
                        idx,
                        kind: "media",
                        key: "src_media",
                        host_display: String::new(),
                        safe_url: None,
                        orig_label: link.label.as_str(),
                    });
                }
            }
            classified.sort_by_key(|s| (s.rank, s.idx));
            let mut host_counts = HashMap::<(&str, &str), usize>::new();
            for s in &classified {
                if s.safe_url.is_some() {
                    *host_counts
                        .entry((s.kind, s.host_display.as_str()))
                        .or_insert(0) += 1;
                }
            }
            let total_links = classified.len();
            out.push_str("<footer class=\"story-links sources\">");
            for (pos, src) in classified.iter().enumerate() {
                let extra = if pos >= 5 {
                    " hidden data-src-extra"
                } else {
                    ""
                };
                let primary = if pos == 0 { " src-primary" } else { "" };
                if let Some(url) = &src.safe_url {
                    let dup_host = host_counts
                        .get(&(src.kind, src.host_display.as_str()))
                        .copied()
                        .unwrap_or(0)
                        > 1;
                    let display =
                        source_link_display(&src.host_display, src.orig_label, url, dup_host);
                    let url_esc = esc(url);
                    let title_attr = if src.orig_label.trim().is_empty() {
                        url_esc.clone()
                    } else {
                        esc(&format!("{} — {url}", src.orig_label.trim()))
                    };
                    let kind_label = t(lang, src.key);
                    write!(
                        out,
                        r#"<a class="src{primary}" data-kind="{}" href="{url_esc}" title="{title_attr}" target="_blank" rel="noopener noreferrer nofollow ugc"{extra}><span class="src-k">{kind_label}</span> {}</a>"#,
                        src.kind,
                        esc(&protect_technical_hyphens(&display))
                    )
                    .unwrap();
                } else {
                    write!(out, "<span>{}</span>", esc(src.orig_label)).unwrap();
                }
            }
            if total_links > 5 {
                let more = t(lang, "src_more").replace("{n}", &(total_links - 5).to_string());
                write!(
                    out,
                    r#"<button type="button" class="src-more" data-src-more>{}</button>"#,
                    esc(&more)
                )
                .unwrap();
            }
            out.push_str("</footer>\n");
        }
        out.push_str("</article>\n");
    }
}

fn source_link_display(host_display: &str, orig_label: &str, url: &str, dup_host: bool) -> String {
    if !dup_host {
        return host_display.to_owned();
    }
    let mut trimmed = orig_label.trim().to_owned();
    for suffix in [
        "论文",
        "官方博客",
        "官方公告",
        "官方",
        "官网",
        "代码仓库",
        "代码",
        "仓库",
        "模型",
        "推文",
        "帖子",
        "X 帖",
        "X帖",
        "报道",
        "Paper",
        "paper",
        "Official Blog",
        "official blog",
        "Official",
        "official",
        "Blog",
        "blog",
        "Code",
        "code",
        "Repo",
        "repo",
        "Model",
        "model",
        "X Post",
        "X post",
        "Post",
        "post",
        "arXiv",
        "GitHub",
        "Hugging Face",
    ] {
        if let Some(rest) = trimmed.strip_suffix(suffix) {
            let candidate = rest
                .trim_end_matches(|c: char| {
                    c.is_whitespace() || matches!(c, '·' | '-' | ':' | '：' | '/')
                })
                .trim();
            trimmed = candidate.to_owned();
        }
    }
    let is_generic = trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case(host_display)
        || matches!(
            trimmed.to_ascii_lowercase().as_str(),
            "arxiv" | "github" | "huggingface" | "hf" | "x" | "twitter" | "link" | "source"
        );
    let label = if !is_generic {
        trimmed
    } else if let Some(seg) = url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut segs| segs.rfind(|s| !s.is_empty()).map(str::to_owned))
        })
        .filter(|s| !s.is_empty())
    {
        seg
    } else if !orig_label.trim().is_empty() {
        orig_label.trim().to_owned()
    } else {
        host_display.to_owned()
    };
    if label.chars().count() > 32 {
        let mut s: String = label.chars().take(31).collect();
        s.push('…');
        s
    } else {
        label
    }
}

fn same_short_date(when: &str, date: NaiveDate) -> bool {
    let Some((m, d)) = when.trim().split_once(['.', '/', '-']) else {
        return false;
    };
    m.parse::<u32>().ok() == Some(date.month()) && d.parse::<u32>().ok() == Some(date.day())
}

fn related_links(text: &str, digest: &Digest) -> String {
    static RELATED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"((?:相关|Related)\s*[：:])((?:\s*[0-9]+(?:\s*[–—-]\s*[0-9]+)?\s*[、,，]?)+)")
            .unwrap()
    });
    static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new("[0-9]+").unwrap());
    RELATED
        .replace_all(text, |m: &regex::Captures<'_>| {
            let links = NUMBER.replace_all(&m[2], |n: &regex::Captures<'_>| {
                digest
                    .items()
                    .find(|it| it.number == n[0])
                    .map_or_else(|| n[0].to_owned(), |it| format!("[{}](#{})", &n[0], it.id))
            });
            format!("{}{links}", &m[1])
        })
        .into_owned()
}

fn read_minutes(digest: &Digest) -> usize {
    let words: usize = digest.items().map(item_word_units).sum();
    ((words as f64 / 400.0).round() as usize).max(1)
}

pub fn render_digest_html(digest: &Digest, lang: Lang, opts: &RenderOptions) -> String {
    let tab_title = compact_tab_title(digest);
    let mut out = page_head(
        lang,
        &tab_title,
        Some(&digest.full_title()),
        assets::READER_CSS,
        false,
    );
    skip_link(&mut out, lang, "");
    Reader {
        digest,
        lang,
        opts,
        prefix: "",
        bilingual: false,
    }
    .render(&mut out);
    write!(
        out,
        "<script>{}</script>\n</body></html>\n",
        assets::READER_JS
    )
    .unwrap();
    out
}

pub fn render_bilingual_html(zh: &Digest, en: &Digest, opts: &RenderOptions) -> String {
    let css = format!("{}{BILINGUAL_CSS}", assets::READER_CSS);
    let tab_title = compact_tab_title(zh);
    let mut out = page_head(Lang::ZhCn, &tab_title, Some(&zh.full_title()), &css, false);
    out.push_str(r#"<input class="sr-only" type="radio" name="digest-language" id="language-zh" aria-label="简体中文">
<input class="sr-only" type="radio" name="digest-language" id="en-language" aria-label="English">
"#);
    for (digest, lang, code, prefix) in [(zh, Lang::ZhCn, "zh", ""), (en, Lang::En, "en", "en-")] {
        writeln!(
            out,
            r#"<div class="lang-pane lang-{code}" data-lang="{lang}" data-lang-pane="{lang}" lang="{lang}" data-title="{}">"#,
            esc(&compact_tab_title(digest))
        )
        .unwrap();
        skip_link(&mut out, lang, prefix);
        Reader {
            digest,
            lang,
            opts,
            prefix,
            bilingual: true,
        }
        .render(&mut out);
        out.push_str("</div>\n");
    }
    write!(
        out,
        "<script>{}</script>\n</body></html>\n",
        assets::READER_JS
    )
    .unwrap();
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveIssue {
    pub date: NaiveDate,
    pub weekly: bool,
    pub href: String,
    pub title: String,
    pub headline: String,
    pub tags: Vec<String>,
    pub item_count: usize,
}

pub fn render_archive_html(issues: &[ArchiveIssue], lang: Lang, opts: &RenderOptions) -> String {
    let mut out = page_head(lang, t(lang, "index_title"), None, assets::INDEX_CSS, false);
    skip_link(&mut out, lang, "");
    out.push_str("<div class=\"mast-sentinel\" aria-hidden=\"true\"></div>\n");
    write!(out, r#"<header class="mast" role="banner"><div class="mast-row"><span class="np"><span class="ai">AI</span> Digest</span><span class="mast-chip">{}</span>"#, i18n::count_issues(lang, issues.len())).unwrap();
    tools(&mut out, lang, "", opts.lang_alt.as_ref(), false);
    out.push_str("</div></header>\n");
    filter_panel(
        &mut out,
        lang,
        "",
        issues.len(),
        &tags_by_count(issues.iter().flat_map(|it| &it.tags)),
        true,
    );
    let dek = if opts.folio_dek.is_empty() {
        t(lang, "motto")
    } else {
        &opts.folio_dek
    };
    writeln!(out, r#"<main id="main" class="paper"><header class="folio"><p class="folio-kicker">{}</p><h1>{}</h1><p class="folio-dek">{}</p></header><div class="quick-bar">"#,
        lang.choose("Archive", "存档"), lang.choose("Daily & Weekly AI Briefings", "每日与每周 AI 简报"), esc(&protect_technical_hyphens(dek))).unwrap();
    for (weekly, kind, key) in [
        (false, "daily", "latest_daily"),
        (true, "weekly", "latest_weekly"),
    ] {
        if let Some(issue) = issues
            .iter()
            .filter(|it| it.weekly == weekly)
            .max_by_key(|it| it.date)
        {
            let href = normalized_safe_url(&issue.href).unwrap_or_else(|| "./".into());
            let href = href
                .rsplit_once('/')
                .filter(|(path, _)| path.rsplit('/').next() == Some(kind))
                .map_or_else(|| href.clone(), |(path, _)| format!("{path}/latest"));
            writeln!(out, r#"<a class="quick-pill" data-kind="{kind}" href="{}"><span class="kicker">{}</span><strong><time datetime="{}">{}</time></strong><small class="num">{}</small></a>"#,
                esc(&href), t(lang, key), issue.date, esc(&i18n::date_long(lang, issue.date)), i18n::count_items(lang, issue.item_count)).unwrap();
        }
    }
    out.push_str("</div><div class=\"timeline\">\n");
    let mut weeks = BTreeMap::<_, Vec<&ArchiveIssue>>::new();
    for issue in issues {
        weeks.entry(issue.date.iso_week()).or_default().push(issue);
    }
    let mut month = None;
    for (i, (week, mut rows)) in weeks.into_iter().rev().enumerate() {
        let latest = rows.iter().map(|it| it.date).max().unwrap();
        let key = (latest.year(), latest.month());
        if month != Some(key) {
            if month.is_some() {
                out.push_str("</section>\n");
            }
            writeln!(
                out,
                r#"<section class="month-group" data-month="{}"><h2 class="month">{}</h2>"#,
                latest.format("%Y-%m"),
                i18n::year_month(lang, latest)
            )
            .unwrap();
            month = Some(key);
        }
        let weekly = rows.iter().filter(|it| it.weekly).count();
        let monday = latest
            .checked_sub_days(chrono::Days::new(u64::from(
                latest.weekday().num_days_from_monday(),
            )))
            .unwrap_or(latest);
        let sunday = monday
            .checked_add_days(chrono::Days::new(6))
            .unwrap_or(latest);
        writeln!(out, r#"<details class="grp" data-week="{}-W{:02}"{}><summary>W{:02} · {} – {} · {} {} · {} {}</summary><ol class="issue-list">"#,
            week.year(), week.week(), if i == 0 { " open" } else { "" }, week.week(), monday.format("%m.%d"), sunday.format("%m.%d"),
            rows.len() - weekly, lang.choose("daily", "日报"), weekly, lang.choose("weekly", "周报")).unwrap();
        rows.sort_by_key(|it| std::cmp::Reverse((it.weekly, it.date)));
        for issue in rows {
            archive_row(&mut out, issue, lang);
        }
        out.push_str("</ol></details>\n");
    }
    if month.is_some() {
        out.push_str("</section>\n");
    }
    out.push_str("</div>\n");
    filter_empty(&mut out, lang, true);
    write!(
        out,
        r#"<footer class="colophon">{}</footer></main><script>{}</script>
</body></html>
"#,
        t(lang, "index_end"),
        assets::INDEX_JS
    )
    .unwrap();
    out
}

fn archive_row(out: &mut String, issue: &ArchiveIssue, lang: Lang) {
    let kind = if issue.weekly { "weekly" } else { "daily" };
    let title = if issue.headline.is_empty() {
        &issue.title
    } else {
        &issue.headline
    };
    let search = format!(
        "{} {} {} {}",
        issue.date,
        issue.title,
        issue.headline,
        issue.tags.join(" ")
    );
    let date = if issue.weekly {
        format!(
            r#"<time datetime="{}">{}</time>"#,
            issue.date,
            t(lang, "edition_weekly")
        )
    } else {
        format!(
            r#"<time datetime="{}">{}</time>"#,
            issue.date,
            issue.date.format("%-m·%-d")
        )
    };
    let href = normalized_safe_url(&issue.href).unwrap_or_else(|| "./".into());
    writeln!(out, r#"<li class="archive-item" data-kind="{kind}" data-tags="{}" data-search="{}"><a class="ai-row" href="{}"><span class="ai-date">{date}</span><span class="ai-wd">{}</span><span class="ai-title">{}</span><span class="ai-cnt num">{}</span></a></li>"#,
        esc(&issue.tags.join("|")), esc(&search), esc(&href), i18n::weekday_short(lang, issue.date), esc(&protect_technical_hyphens(title)), i18n::count_items(lang, issue.item_count)).unwrap();
}

pub fn render_404_html() -> String {
    let mut out = page_head(
        Lang::ZhCn,
        "404 · AI Digest",
        None,
        assets::NOT_FOUND_CSS,
        true,
    );
    out.push_str(r#"<main class="nf"><p class="kicker num">404</p><h1>这一页不在刊中<span lang="en">This page isn't in any issue</span></h1>
<nav aria-label="期刊导航">
<a href="/latest"><span>最新日报 · <span lang="en">Latest daily</span></span><span aria-hidden="true">→</span></a>
<a href="/weekly/latest"><span>最新周报 · <span lang="en">Latest weekly</span></span><span aria-hidden="true">→</span></a>
<a href="/"><span>存档 · <span lang="en">Archive</span></span><span aria-hidden="true">→</span></a>
</nav></main></body></html>
"#);
    out
}

#[derive(Clone, Debug, Default)]
pub struct NavOptions {
    pub lang: Lang,
    pub home: Option<LinkItem>,
    pub previous: Option<LinkItem>,
    pub next: Option<LinkItem>,
    pub weekly: Option<LinkItem>,
    pub lang_alt: Option<LangAlt>,
    pub noindex: bool,
}

static BRAND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(<a\s+class="np-link"\s+href=")[^"]*("\s+data-brand-home>)"#).unwrap()
});
static PANE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)<div\s+class="lang-pane\s+lang-(zh|en)""#).unwrap());
static HEADER_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)</header\s*>").unwrap());
static BODY_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)</body\s*>").unwrap());
static HEAD_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)</head\s*>").unwrap());
static LEGACY_END: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)<div class="end">[^<]*</div>"#).unwrap());
static ROBOTS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)<meta\s+name=['"]robots['"][^>]*>"#).unwrap());

pub fn strip_injections(html: &str) -> String {
    let markers = [
        "digest-nav:top",
        "digest-nav:bottom",
        "digest-meta",
        "digest-lock",
        "digest-lock-btn",
        "digest-lock-js",
        "digest-nav-css",
        "digest-perf",
        "digest-lang-btn",
        "digest-nav-top",
        "digest-nav-bottom",
    ];
    let clean = markers
        .iter()
        .fold(html.to_owned(), |doc, marker| strip_marked(&doc, marker));
    BRAND.replace_all(&clean, "${1}./${2}").into_owned()
}

pub fn inject_nav(html: &str, options: &NavOptions) -> String {
    let mut clean = strip_injections(html);
    if options.noindex {
        clean = ROBOTS.replace_all(&clean, "").into_owned();
        if let Some(head) = HEAD_END.find(&clean) {
            clean.insert_str(head.start(), r#"<!-- digest-meta:start --><meta name="robots" content="noindex,nofollow"><!-- digest-meta:end -->"#);
        }
    }
    let panes: Vec<_> = PANE
        .captures_iter(&clean)
        .map(|m| {
            (
                m.get(0).unwrap().start(),
                if m[1].eq_ignore_ascii_case("en") {
                    Lang::En
                } else {
                    Lang::ZhCn
                },
            )
        })
        .collect();
    if panes.len() == 2 {
        let split = panes[1].0;
        return nav_fragment(&clean[..split], options, panes[0].1, true)
            + &nav_fragment(&clean[split..], options, panes[1].1, true);
    }
    nav_fragment(&clean, options, options.lang, false)
}

fn nav_fragment(html: &str, opts: &NavOptions, lang: Lang, bilingual: bool) -> String {
    let prefix = if bilingual && lang == Lang::En {
        "en-"
    } else {
        ""
    };
    let nav_link = |link: Option<&LinkItem>, key: &str, role: &str, symbol: &str, top: bool| {
        let label = esc(t(lang, key));
        let class = format!("nav-{role}");
        let short_lbl = match role {
            "prev" => lang.choose("Prev", "上一期"),
            "next" => lang.choose("Next", "下一期"),
            "index" => lang.choose("Archive", "归档"),
            _ => "",
        };
        if let Some((link, url)) =
            link.and_then(|link| normalized_safe_url(&link.url).map(|url| (link, url)))
        {
            let url = esc(&prefix_fragment(url, prefix));
            let detail = esc(&link.label);
            if top {
                let semantics = if matches!(role, "prev" | "next") {
                    format!(r#" rel="{role}" data-nav="{role}""#)
                } else {
                    String::new()
                };
                let body = match role {
                    "week" => label.clone(),
                    "index" => format!(
                        r#"<span aria-hidden="true">{symbol}</span><span class="nav-lbl">{short_lbl}</span>"#
                    ),
                    "prev" => {
                        let txt = if detail.is_empty() {
                            short_lbl
                        } else {
                            &detail
                        };
                        format!(
                            r#"<span aria-hidden="true">{symbol}</span><span data-short-date>{txt}</span>"#
                        )
                    }
                    "next" => {
                        let txt = if detail.is_empty() {
                            short_lbl
                        } else {
                            &detail
                        };
                        format!(
                            r#"<span data-short-date>{txt}</span><span aria-hidden="true">{symbol}</span>"#
                        )
                    }
                    _ => format!(r#"<span aria-hidden="true">{symbol}</span>"#),
                };
                format!(
                    r#"<a class="{class}" href="{url}"{semantics} aria-label="{label}">{body}</a>"#
                )
            } else {
                format!(
                    r#"<a class="{class}" href="{url}"><span>{symbol} {label}</span><span>{detail}</span></a>"#
                )
            }
        } else if top && matches!(role, "prev" | "next") {
            let body = if role == "prev" {
                format!(
                    r#"<span aria-hidden="true">{symbol}</span><span data-short-date>{short_lbl}</span>"#
                )
            } else {
                format!(
                    r#"<span data-short-date>{short_lbl}</span><span aria-hidden="true">{symbol}</span>"#
                )
            };
            format!(
                r#"<span class="{class}" aria-disabled="true" aria-label="{label}">{body}</span>"#
            )
        } else {
            format!(
                r#"<span class="{class}" aria-disabled="true" aria-label="{label}">{symbol}</span>"#
            )
        }
    };
    let links = |top| {
        let prev = nav_link(opts.previous.as_ref(), "nav_prev", "prev", "‹", top);
        let next = nav_link(opts.next.as_ref(), "nav_next", "next", "›", top);
        let home = if top {
            nav_link(opts.home.as_ref(), "nav_index", "index", "⊞", top)
        } else {
            String::new()
        };
        let week = opts.weekly.as_ref().map_or_else(String::new, |link| {
            nav_link(Some(link), "nav_week", "week", "", top)
        });
        format!("{prev}{home}{next}{week}")
    };
    let label = esc(t(lang, "nav_aria"));
    let top = format!(
        r#"<!-- digest-nav:top:start --><nav class="issue-nav top num" aria-label="{label}">{}</nav><!-- digest-nav:top:end -->"#,
        links(true)
    );
    let bottom = format!(
        r#"<!-- digest-nav:bottom:start --><nav class="issue-nav bottom num" aria-label="{label}">{}</nav><!-- digest-nav:bottom:end -->"#,
        links(false)
    );
    let mut out = if let Some(href) = opts
        .home
        .as_ref()
        .and_then(|link| normalized_safe_url(&link.url))
    {
        let href = esc(&prefix_fragment(href, prefix));
        BRAND
            .replace_all(html, |m: &regex::Captures<'_>| {
                format!("{}{href}{}", &m[1], &m[2])
            })
            .into_owned()
    } else {
        html.to_owned()
    };
    if let Some(pos) = out
        .find(TOP_SLOT)
        .map(|pos| pos + TOP_SLOT.len())
        .or_else(|| HEADER_END.find(&out).map(|m| m.start()))
    {
        out.insert_str(pos, &top);
    }
    if let Some(pos) = out
        .find(BOTTOM_SLOT)
        .map(|pos| pos + BOTTOM_SLOT.len())
        .or_else(|| LEGACY_END.find(&out).map(|m| m.end()))
        .or_else(|| BODY_END.find(&out).map(|m| m.start()))
    {
        out.insert_str(pos, &bottom);
    }
    if let Some(alt) = opts.lang_alt.as_ref().filter(|_| !bilingual) {
        let control = lang_control(lang, Some(alt), false);
        if !control.is_empty() && !out.contains(" data-lang-link") {
            if let Some(i) = out.find(r#"<button type="button" class="ctrl-btn theme-btn"#) {
                out.insert_str(
                    i,
                    &format!("<!-- digest-lang-btn:start -->{control}<!-- digest-lang-btn:end -->"),
                );
            }
        }
    }
    out
}
