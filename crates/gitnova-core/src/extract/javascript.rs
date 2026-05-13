use super::FileExtraction;
use crate::parser::ParsedFile;

pub fn extract(parsed: &ParsedFile) -> FileExtraction {
    super::typescript::extract(parsed)
}
