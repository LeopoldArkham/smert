use crate::error::Result;
use serde::{Deserialize, Serialize};

const DATA_FILE_EN: &str = "./definitions_en.json";
const DATA_FILE_FR: &str = "./definitions_fr.json";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Definition {
    pub word:        String,
    /// Translation of the word into the other language.
    /// Old JSON files stored a grammatical category here under the key
    /// "part_of_speech"; the alias lets us load those files transparently
    /// until backfill_translations() overwrites them with real translations.
    #[serde(alias = "part_of_speech")]
    pub translation: String,
    pub definition:  String,
}

fn data_file(lang: &str) -> &'static str {
    if lang == "french" { DATA_FILE_FR } else { DATA_FILE_EN }
}

pub fn load_definitions(lang: &str) -> Result<Vec<Definition>> {
    let path = data_file(lang);
    if !std::path::Path::new(path).exists() {
        return Ok(vec![]);
    }
    let raw = std::fs::read_to_string(path)?;
    serde_json::from_str::<Vec<Definition>>(&raw).map_err(Into::into)
}

pub fn delete_definition(lang: &str, index: usize) -> Result<()> {
    let mut defs = load_definitions(lang)?;
    if index < defs.len() {
        defs.remove(index);
        std::fs::write(data_file(lang), serde_json::to_string_pretty(&defs)?)?;
    }
    Ok(())
}

/// Fetch the definition + translation and append to the JSON file.
pub async fn fetch_and_add(word: &str, lang: &str) -> Result<()> {
    let def = if lang == "french" {
        fetch_fr(word).await?
    } else {
        fetch_en(word).await?
    };
    let mut defs = load_definitions(lang)?;
    defs.push(def);
    std::fs::write(data_file(lang), serde_json::to_string_pretty(&defs)?)?;
    Ok(())
}


/// Pick one definition at random (returns a Vec for consistency with other resources).
pub fn pick_one(defs: &[Definition]) -> Vec<Definition> {
    use rand::seq::IndexedRandom;
    let mut rng = rand::rng();
    defs.choose(&mut rng).cloned().into_iter().collect()
}

// ── Translation via English Wiktionary ───────────────────────────────────────
//
// Both directions go through en.wiktionary.org:
//   EN → FR  parse the ==Translations== section for a "* French:" entry.
//   FR → EN  the French word's en.wiktionary entry has English-language
//            definitions — take the first one as the translation.

async fn fetch_translation(word: &str, from: &str, to: &str) -> Result<String> {
    let content = fetch_en_wiktionary(word).await?;
    if from == "en" && to == "fr" {
        extract_fr_from_en_wiktionary(word, &content)
    } else {
        extract_en_gloss_for_fr_word(word, &content)
    }
}

/// Fetch raw wikitext for `word` from en.wiktionary.org.
async fn fetch_en_wiktionary(word: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct ApiResp  { query: ApiQuery }
    #[derive(Deserialize)]
    struct ApiQuery { pages: Vec<ApiPage> }
    #[derive(Deserialize)]
    struct ApiPage  { revisions: Option<Vec<ApiRev>> }
    #[derive(Deserialize)]
    struct ApiRev   { slots: ApiSlots }
    #[derive(Deserialize)]
    struct ApiSlots { main: ApiMain }
    #[derive(Deserialize)]
    struct ApiMain  { content: String }

    let url = format!(
        "https://en.wiktionary.org/w/api.php?action=query&format=json\
         &titles={}&redirects=1&prop=revisions&rvprop=content\
         &rvslots=main&formatversion=2",
        urlencoding::encode(word),
    );
    let client = reqwest::Client::builder().user_agent("smert/1.0").build()?;
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(format!("en.wiktionary returned {}", resp.status()).into());
    }
    let data: ApiResp = resp.json().await?;
    data.query.pages.into_iter()
        .next()
        .and_then(|p| p.revisions)
        .and_then(|mut r| r.pop())
        .map(|r| r.slots.main.content)
        .ok_or_else(|| "no wiktionary entry found".into())
}

/// For an English word: find the first French translation in the Translations
/// section. Handles `{{t+|fr|word}}` and `{{t|fr|word|...}}` templates.
fn extract_fr_from_en_wiktionary(word: &str, wikitext: &str) -> Result<String> {
    let _ = word;
    // Locate any Translations heading (== or === or ====)
    let trans_pos = wikitext.find("Translations")
        .ok_or("no Translations section found")?;
    let section = &wikitext[trans_pos..];

    // Find a line that starts with "* French:"
    let fr_line = section.lines()
        .find(|l| l.trim_start().starts_with("* French:"))
        .ok_or("no French entry in Translations section")?;

    // Extract the first {{t+|fr|word}} or {{t|fr|word}} template.
    // Format: {{t[+]|fr|WORD|optional-gender...}}
    let mut rest = fr_line;
    while let Some(pos) = rest.find("{{t") {
        rest = &rest[pos + 3..]; // skip "{{t"
        // skip optional "+"
        if rest.starts_with('+') { rest = &rest[1..]; }
        // skip "|fr|"
        if !rest.starts_with("|fr|") { continue; }
        rest = &rest[4..];
        // word ends at the next '|' or '}}'
        let end = rest.find(|c: char| c == '|' || c == '}').unwrap_or(rest.len());
        let candidate = rest[..end].trim();
        if !candidate.is_empty() {
            return Ok(candidate.to_string());
        }
    }
    Err("could not extract French translation from template".into())
}

/// Return the byte offset of the next `==Language==` heading in `s`
/// (exactly two leading `=`, not three or more).
fn next_language_section(s: &str) -> Option<usize> {
    let mut haystack = s;
    let mut base = 0;
    while let Some(rel) = haystack.find("\n==") {
        let after = rel + 3; // byte after the two '='
        let third = haystack.as_bytes().get(after).copied();
        if third != Some(b'=') {
            return Some(base + rel);
        }
        base += rel + 1;
        haystack = &haystack[rel + 1..];
    }
    None
}

/// For a French word: find the `==French==` section on en.wiktionary and
/// return its first English-language definition line as the gloss.
fn extract_en_gloss_for_fr_word(word: &str, wikitext: &str) -> Result<String> {
    let _ = word;
    let fr_pos = wikitext.find("==French==")
        .ok_or("no French section on en.wiktionary")?;
    let after = wikitext[fr_pos..].find('\n').map(|p| fr_pos + p + 1).unwrap_or(wikitext.len());
    let section = &wikitext[after..];
    // End at the next TOP-LEVEL language section (==X==, not ===X===).
    // We must not stop at sub-headings, so we verify the third char is not '='.
    let end = next_language_section(section).unwrap_or(section.len());
    let section = &section[..end];

    // First "# " definition line (not sub-definition "## ")
    let def = section.lines()
        .find(|l| l.starts_with("# ") && !l.starts_with("## "))
        .ok_or("no definition line in French section")?;

    let cleaned = clean_wikitext(&def[2..]);
    // Take only the first comma- or semicolon-delimited term as a concise gloss.
    let gloss = cleaned.split([',', ';']).next().unwrap_or(&cleaned).trim().to_string();
    if gloss.is_empty() {
        return Err("empty gloss after cleaning".into());
    }
    // Trim a trailing period if present.
    Ok(gloss.trim_end_matches('.').trim().to_string())
}

// ── English: Free Dictionary API ─────────────────────────────────────────────

async fn fetch_en(word: &str) -> Result<Definition> {
    let url = format!("https://api.dictionaryapi.dev/api/v2/entries/en/{}", word);

    let client = reqwest::Client::builder()
        .user_agent("smert/1.0")
        .build()?;

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(format!("dictionary API returned {}", resp.status()).into());
    }

    #[derive(Deserialize)]
    struct Entry { meanings: Vec<Meaning> }
    #[derive(Deserialize)]
    struct Meaning { definitions: Vec<Def> }
    #[derive(Deserialize)]
    struct Def { definition: String }

    let entries: Vec<Entry> = resp.json().await?;
    let defs: Vec<String> = entries.into_iter()
        .flat_map(|e| e.meanings)
        .flat_map(|m| m.definitions)
        .map(|d| d.definition)
        .collect();
    if defs.is_empty() {
        return Err("no definitions found".into());
    }

    // A missing translation should not block adding the word: Wiktionary's
    // Translations table often omits French entirely. Fall back to empty.
    let translation = fetch_translation(word, "en", "fr").await.unwrap_or_default();

    Ok(Definition {
        word:        word.to_string(),
        translation,
        definition:  defs.join("\n"),
    })
}

// ── French: Wiktionnaire MediaWiki API (wikitext) ────────────────────────────

async fn fetch_fr(word: &str) -> Result<Definition> {
    #[derive(Deserialize)]
    struct ApiResp  { query: ApiQuery }
    #[derive(Deserialize)]
    struct ApiQuery { pages: Vec<ApiPage> }
    #[derive(Deserialize)]
    struct ApiPage  { revisions: Option<Vec<ApiRev>> }
    #[derive(Deserialize)]
    struct ApiRev   { slots: ApiSlots }
    #[derive(Deserialize)]
    struct ApiSlots { main: ApiMain }
    #[derive(Deserialize)]
    struct ApiMain  { content: String }

    let url = format!(
        "https://fr.wiktionary.org/w/api.php?action=query&format=json\
         &titles={}&redirects=1&prop=revisions&rvprop=content\
         &rvslots=main&formatversion=2",
        urlencoding::encode(word),
    );

    let client = reqwest::Client::builder()
        .user_agent("smert/1.0")
        .build()?;

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(format!("Wiktionnaire returned {}", resp.status()).into());
    }

    let data: ApiResp = resp.json().await?;
    let content = data.query.pages.into_iter()
        .next()
        .and_then(|p| p.revisions)
        .and_then(|mut r| r.pop())
        .map(|r| r.slots.main.content)
        .ok_or("page not found or has no content")?;

    let definition = parse_fr_definition(word, &content)?;
    // A missing translation should not block adding the word; fall back to empty.
    let translation = fetch_translation(word, "fr", "en").await.unwrap_or_default();

    Ok(Definition {
        word: word.to_string(),
        translation,
        definition,
    })
}

/// Extract first definitions from the `== {{langue|fr}} ==` section.
fn parse_fr_definition(word: &str, wikitext: &str) -> Result<String> {
    let _ = word;
    let fr_pos = wikitext.find("{{langue|fr}}")
        .ok_or("no French section in article")?;

    let after_header = wikitext[fr_pos..]
        .find('\n')
        .map(|p| fr_pos + p + 1)
        .unwrap_or(wikitext.len());

    let section = &wikitext[after_header..];
    let section_end = section.find("\n== {{langue|").unwrap_or(section.len());
    let section = &section[..section_end];

    let defs: Vec<String> = section.lines()
        .filter(|l| l.starts_with("# ") && !l.starts_with("## "))
        .map(|l| clean_wikitext(&l[2..]))
        .filter(|s| !s.is_empty())
        .collect();

    if defs.is_empty() {
        return Err("no definition lines found".into());
    }

    Ok(defs.join("\n"))
}

/// Strip wiki markup: [[links]], {{templates}}, bold/italic markers.
fn clean_wikitext(text: &str) -> String {
    let mut out    = String::with_capacity(text.len());
    let mut chars  = text.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '[' if chars.peek() == Some(&'[') => {
                chars.next();
                let mut inner = String::new();
                loop {
                    match chars.next() {
                        None       => break,
                        Some(']') => {
                            if chars.peek() == Some(&']') { chars.next(); break; }
                            inner.push(']');
                        }
                        Some(ch) => inner.push(ch),
                    }
                }
                let label = inner.rsplit('|').next().unwrap_or(&inner);
                out.push_str(label);
            }
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                let mut body  = String::new();
                let mut depth = 2usize;
                while let Some(ch) = chars.next() {
                    match ch {
                        '{' => { depth += 1; body.push(ch); }
                        '}' => {
                            depth -= 1;
                            if depth == 0 { break; }
                            body.push(ch);
                        }
                        _   => body.push(ch),
                    }
                }
                let name = body.split('|').next().unwrap_or("").trim();
                if let Some(label) = fr_label(name) {
                    out.push_str(label);
                    out.push(' ');
                }
            }
            '\'' if chars.peek() == Some(&'\'') => {
                chars.next();
                if chars.peek() == Some(&'\'') { chars.next(); }
            }
            _ => out.push(c),
        }
    }

    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn fr_label(name: &str) -> Option<&'static str> {
    match name {
        "fig" | "figuré" | "au figuré"             => Some("(Figuré)"),
        "fam" | "familier"                          => Some("(Familier)"),
        "pop" | "populaire"                         => Some("(Populaire)"),
        "arg" | "argot"                             => Some("(Argot)"),
        "vx"  | "vieilli"                           => Some("(Vieilli)"),
        "litt"| "littéraire" | "soutenu"            => Some("(Littéraire)"),
        "péj" | "péjoratif"                         => Some("(Péjoratif)"),
        "iron"| "ironique"                          => Some("(Ironique)"),
        "euphem" | "euphémisme"                     => Some("(Euphémisme)"),
        "vulg"| "vulgaire"                          => Some("(Vulgaire)"),
        "rare"                                      => Some("(Rare)"),
        "très rare"                                 => Some("(Très rare)"),
        "dial"| "dialectal" | "régional"            => Some("(Régional)"),
        "belg"                                      => Some("(Belgique)"),
        "can"                                       => Some("(Canada)"),
        "suisse"                                    => Some("(Suisse)"),
        "afr"                                       => Some("(Afrique)"),
        "méd"  | "médecine"                         => Some("(Médecine)"),
        "physio"| "physiologie"                     => Some("(Physiologie)"),
        "chir"  | "chirurgie"                       => Some("(Chirurgie)"),
        "pharm" | "pharmacologie"                   => Some("(Pharmacologie)"),
        "anat"  | "anatomie"                        => Some("(Anatomie)"),
        "path"  | "pathologie"                      => Some("(Pathologie)"),
        "psycho"| "psychologie"                     => Some("(Psychologie)"),
        "bio"   | "biologie"                        => Some("(Biologie)"),
        "bot"   | "botanique"                       => Some("(Botanique)"),
        "zool"  | "zoologie"                        => Some("(Zoologie)"),
        "chim"  | "chimie"                          => Some("(Chimie)"),
        "phys"  | "physique"                        => Some("(Physique)"),
        "math"  | "mathématiques"                   => Some("(Mathématiques)"),
        "inform"| "informatique"                    => Some("(Informatique)"),
        "tech"  | "technique"                       => Some("(Technique)"),
        "ind"   | "industrie"                       => Some("(Industrie)"),
        "agr"   | "agriculture"                     => Some("(Agriculture)"),
        "arch"  | "architecture"                    => Some("(Architecture)"),
        "arts"                                      => Some("(Arts)"),
        "mus"   | "musique"                         => Some("(Musique)"),
        "dr"    | "droit"                           => Some("(Droit)"),
        "comm"  | "commerce"                        => Some("(Commerce)"),
        "écon"  | "économie"                        => Some("(Économie)"),
        "fin"   | "finance"                         => Some("(Finance)"),
        "pol"   | "politique"                       => Some("(Politique)"),
        "mil"   | "militaire"                       => Some("(Militaire)"),
        "mar"   | "marine"                          => Some("(Marine)"),
        "géo"   | "géographie"                      => Some("(Géographie)"),
        "géol"  | "géologie"                        => Some("(Géologie)"),
        "hist"  | "histoire"                        => Some("(Histoire)"),
        "ling"  | "linguistique"                    => Some("(Linguistique)"),
        "gramm" | "grammaire"                       => Some("(Grammaire)"),
        "philo" | "philosophie"                     => Some("(Philosophie)"),
        "relig" | "religion"                        => Some("(Religion)"),
        "théol" | "théologie"                       => Some("(Théologie)"),
        "myth"  | "mythologie"                      => Some("(Mythologie)"),
        "sport"                                     => Some("(Sport)"),
        "jeux"                                      => Some("(Jeux)"),
        "astro" | "astronomie"                      => Some("(Astronomie)"),
        "météo" | "météorologie"                    => Some("(Météorologie)"),
        "électr"| "électricité"                     => Some("(Électricité)"),
        "mécan" | "mécanique"                       => Some("(Mécanique)"),
        _                                           => None,
    }
}
