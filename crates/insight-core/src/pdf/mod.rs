mod extractor;

pub use extractor::{
    char_offset_to_page, extract_text, extract_text_from_bytes, rasterize_page, ExtractedDocument,
    OcrTask, PageDecision, PageExtraction, DIGITAL_TEXT_THRESHOLD, RASTER_DPI,
};
