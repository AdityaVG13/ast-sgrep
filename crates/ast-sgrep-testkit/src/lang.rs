use ast_sgrep_lang::{ExtractionResult, Language, ParserRegistry};

pub fn parse(lang: Language, source: &str) -> ExtractionResult {
    ParserRegistry::new().parse(lang, source).expect("parse")
}
