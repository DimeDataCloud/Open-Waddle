//! Reading documents as text (PDF, Word, PowerPoint, Excel and CSV) and making
//! simple new ones (Word, Excel, CSV, Markdown). The writers produce minimal
//! Office Open XML packages that Word and Excel open as-is.

use anyhow::{bail, Context};
use quick_xml::events::Event;
use std::io::{Cursor, Read, Write};
use std::path::Path;

/// The most text a document read returns.
const MAX_TEXT: usize = 20_000;
const MAX_ROWS: usize = 200;
const MAX_COLS: usize = 30;

/// Reads a document as text. `pages` (1-based, inclusive) limits PDF pages and slides.
pub fn read_document(path: &Path, pages: Option<(usize, usize)>) -> anyhow::Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    read_bytes(&name, &bytes, pages)
}

/// Like `read_document`, for bytes that came from somewhere else (a Drive download).
pub fn read_bytes(name: &str, bytes: &[u8], pages: Option<(usize, usize)>) -> anyhow::Result<String> {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
    let text = match ext.as_str() {
        "pdf" => pdf_text(bytes, pages)?,
        "docx" | "docm" => docx_text(bytes)?,
        "pptx" => pptx_text(bytes, pages)?,
        "xlsx" | "xlsm" | "xls" | "ods" => sheet_text(bytes)?,
        "csv" | "tsv" => csv_text(&String::from_utf8_lossy(bytes)),
        "doc" | "ppt" => bail!("old .{ext} files can't be read; save it as .{ext}x first"),
        _ => {
            if bytes.iter().take(4096).any(|b| *b == 0) {
                bail!("`{name}` isn't a document Waddle can read (it looks like a binary file)");
            }
            String::from_utf8_lossy(bytes).into_owned()
        }
    };
    let total = text.chars().count();
    if total > MAX_TEXT {
        return Ok(format!("{}\n[… cut: {MAX_TEXT} of {total} characters; ask for later pages or a part]", text.chars().take(MAX_TEXT).collect::<String>()));
    }
    Ok(if text.trim().is_empty() { "(no text found: it may be a scan or only pictures)".into() } else { text })
}

fn pdf_text(bytes: &[u8], pages: Option<(usize, usize)>) -> anyhow::Result<String> {
    // The PDF parser can panic on damaged files; a bad file shouldn't take Waddle down.
    let bytes = bytes.to_vec();
    let by_page = std::panic::catch_unwind(move || pdf_extract::extract_text_from_mem_by_pages(&bytes))
        .map_err(|_| anyhow::anyhow!("this PDF couldn't be read (it may be damaged or encrypted)"))?
        .map_err(|e| anyhow::anyhow!("this PDF couldn't be read: {e}"))?;
    let count = by_page.len();
    let (from, to) = pages.unwrap_or((1, count)).clamp_range(count);
    let mut out = format!("PDF, {count} page(s).");
    for (i, page) in by_page.iter().enumerate().take(to).skip(from - 1) {
        out.push_str(&format!("\n\n— Page {} —\n{}", i + 1, tidy(page)));
    }
    Ok(out)
}

trait ClampRange {
    fn clamp_range(self, count: usize) -> (usize, usize);
}

impl ClampRange for (usize, usize) {
    fn clamp_range(self, count: usize) -> (usize, usize) {
        let from = self.0.max(1).min(count.max(1));
        (from, self.1.max(from).min(count))
    }
}

/// Squashes runs of blank lines and trailing spaces.
fn tidy(s: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for line in s.lines().map(str::trim_end) {
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

fn zip_file(bytes: &[u8], name: &str) -> anyhow::Result<String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).context("not a valid Office file")?;
    let mut f = archive.by_name(name).with_context(|| format!("{name} is missing"))?;
    let mut s = String::new();
    f.read_to_string(&mut s)?;
    Ok(s)
}

/// Text from WordprocessingML or DrawingML: `t` elements, with paragraph, tab and break marks.
fn xml_text(xml: &str, para: &str, text: &str) -> String {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut out = String::new();
    let mut in_text = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.local_name().as_ref() == text => in_text = true,
            Ok(Event::End(e)) if e.local_name().as_ref() == text => in_text = false,
            Ok(Event::End(e)) if e.local_name().as_ref() == para => out.push('\n'),
            Ok(Event::Empty(e)) if matches!(e.local_name().as_ref(), "tab") => out.push('\t'),
            Ok(Event::Empty(e)) if matches!(e.local_name().as_ref(), "br" | "cr") => out.push('\n'),
            Ok(Event::Text(t)) if in_text => out.push_str(&t.xml10_content()),
            Ok(Event::GeneralRef(r)) if in_text => {
                let entity: &str = r.as_ref();
                out.push_str(match entity {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "quot" => "\"",
                    "apos" => "'",
                    _ => "",
                });
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    tidy(&out)
}

fn docx_text(bytes: &[u8]) -> anyhow::Result<String> {
    Ok(xml_text(&zip_file(bytes, "word/document.xml")?, "p", "t"))
}

fn pptx_text(bytes: &[u8], pages: Option<(usize, usize)>) -> anyhow::Result<String> {
    let archive = zip::ZipArchive::new(Cursor::new(bytes)).context("not a valid PowerPoint file")?;
    let mut slides: Vec<(usize, String)> = archive
        .file_names()
        .filter_map(|n| n.strip_prefix("ppt/slides/slide")?.strip_suffix(".xml")?.parse().ok().map(|i: usize| (i, n.to_string())))
        .collect();
    slides.sort();
    let count = slides.len();
    let (from, to) = pages.unwrap_or((1, count)).clamp_range(count);
    let mut out = format!("Presentation, {count} slide(s).");
    for (i, name) in slides.iter().take(to).skip(from - 1) {
        out.push_str(&format!("\n\n— Slide {i} —\n{}", xml_text(&zip_file(bytes, name)?, "p", "t")));
    }
    Ok(out)
}

fn sheet_text(bytes: &[u8]) -> anyhow::Result<String> {
    use calamine::Reader;
    let mut wb = calamine::open_workbook_auto_from_rs(Cursor::new(bytes.to_vec())).context("not a spreadsheet Waddle can read")?;
    let mut out = String::new();
    for name in wb.sheet_names().to_owned() {
        let Ok(range) = wb.worksheet_range(&name) else { continue };
        let (rows, cols) = range.get_size();
        out.push_str(&format!("Sheet \"{name}\" ({rows} rows × {cols} columns):\n"));
        for row in range.rows().take(MAX_ROWS) {
            let cells: Vec<String> = row.iter().take(MAX_COLS).map(|c| c.to_string()).collect();
            out.push_str(&cells.join(" | "));
            out.push('\n');
        }
        if rows > MAX_ROWS {
            out.push_str(&format!("[… {} more rows]\n", rows - MAX_ROWS));
        }
        out.push('\n');
    }
    Ok(out.trim_end().to_string())
}

fn csv_text(text: &str) -> String {
    let rows = parse_csv(text);
    let mut out: Vec<String> = rows.iter().take(MAX_ROWS).map(|r| r.iter().take(MAX_COLS).cloned().collect::<Vec<_>>().join(" | ")).collect();
    if rows.len() > MAX_ROWS {
        out.push(format!("[… {} more rows]", rows.len() - MAX_ROWS));
    }
    format!("Table, {} rows:\n{}", rows.len(), out.join("\n"))
}

/// A small CSV reader: commas or tabs, quoted fields with doubled quotes.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let sep = if text.lines().next().is_some_and(|l| l.contains('\t') && !l.contains(',')) { '\t' } else { ',' };
    let mut rows = vec![];
    let mut row = vec![];
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            ('"', true) => quoted = false,
            ('"', false) if field.is_empty() => quoted = true,
            (c, false) if c == sep => row.push(std::mem::take(&mut field)),
            ('\r', false) => {}
            ('\n', false) => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            (c, _) => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn escape(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == '\t')
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn package(parts: &[(&str, String)]) -> anyhow::Result<Vec<u8>> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in parts {
            zip.start_file(*name, opts)?;
            zip.write_all(body.as_bytes())?;
        }
        zip.finish()?;
    }
    Ok(buf.into_inner())
}

const XML_HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;

/// A Word document: `# ` lines become headings, `- ` lines bullets, other lines paragraphs.
pub fn make_docx(content: &str) -> anyhow::Result<Vec<u8>> {
    let mut body = String::new();
    for line in content.lines() {
        let (text, bold, size) = match line.trim_start() {
            l if l.starts_with("# ") => (l[2..].to_string(), true, Some(36)),
            l if l.starts_with("## ") => (l[3..].to_string(), true, Some(28)),
            l if l.starts_with("- ") || l.starts_with("* ") => (format!("• {}", &l[2..]), false, None),
            l => (l.to_string(), false, None),
        };
        let mut props = String::new();
        if bold {
            props.push_str("<w:b/>");
        }
        if let Some(sz) = size {
            props.push_str(&format!("<w:sz w:val=\"{sz}\"/>"));
        }
        let rpr = if props.is_empty() { String::new() } else { format!("<w:rPr>{props}</w:rPr>") };
        body.push_str(&format!("<w:p><w:r>{rpr}<w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>", escape(&text)));
    }
    package(&[
        (
            "[Content_Types].xml",
            format!(
                r#"{XML_HEAD}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#
            ),
        ),
        (
            "_rels/.rels",
            format!(
                r#"{XML_HEAD}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#
            ),
        ),
        (
            "word/document.xml",
            format!(r#"{XML_HEAD}<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#),
        ),
    ])
}

fn column(i: usize) -> String {
    let mut n = i + 1;
    let mut s = String::new();
    while n > 0 {
        let r = (n - 1) % 26;
        s.insert(0, (b'A' + r as u8) as char);
        n = (n - 1) / 26;
    }
    s
}

/// An Excel workbook with one sheet, from CSV text. Numbers stay numbers.
pub fn make_xlsx(csv: &str) -> anyhow::Result<Vec<u8>> {
    let rows = parse_csv(csv);
    let mut data = String::new();
    for (r, row) in rows.iter().enumerate() {
        data.push_str(&format!("<row r=\"{}\">", r + 1));
        for (c, cell) in row.iter().enumerate() {
            let at = format!("{}{}", column(c), r + 1);
            let t = cell.trim();
            if !t.is_empty() && t.parse::<f64>().is_ok_and(|n| n.is_finite()) {
                data.push_str(&format!("<c r=\"{at}\"><v>{t}</v></c>"));
            } else if !cell.is_empty() {
                data.push_str(&format!("<c r=\"{at}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>", escape(cell)));
            }
        }
        data.push_str("</row>");
    }
    package(&[
        (
            "[Content_Types].xml",
            format!(
                r#"{XML_HEAD}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#
            ),
        ),
        (
            "_rels/.rels",
            format!(
                r#"{XML_HEAD}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/workbook.xml",
            format!(
                r#"{XML_HEAD}<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            format!(
                r#"{XML_HEAD}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/worksheets/sheet1.xml",
            format!(r#"{XML_HEAD}<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{data}</sheetData></worksheet>"#),
        ),
    ])
}

/// The bytes of a new document of `kind` (docx, xlsx, csv or md).
pub fn make_document(kind: &str, content: &str) -> anyhow::Result<Vec<u8>> {
    match kind.trim().to_lowercase().as_str() {
        "docx" | "word" => make_docx(content),
        "xlsx" | "excel" => make_xlsx(content),
        "csv" | "md" | "markdown" | "txt" => Ok(content.as_bytes().to_vec()),
        other => bail!("can't make a `{other}` document; use docx, xlsx, csv or md"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-page PDF with real text, built by hand with a correct cross-reference table.
    fn tiny_pdf(lines: &[&str]) -> Vec<u8> {
        let stream: String = lines.iter().enumerate().map(|(i, l)| format!("BT /F1 14 Tf 20 {} Td ({l}) Tj ET\n", 120 - i * 20)).collect();
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 144] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{stream}endstream", stream.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = vec![];
        for (i, o) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for off in offsets {
            pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
        pdf
    }

    #[test]
    fn reads_pdf_text_by_page() {
        let text = read_bytes("report.pdf", &tiny_pdf(&["Quarterly report", "Revenue rose 12%"]), None).unwrap();
        assert!(text.starts_with("PDF, 1 page(s)."), "{text}");
        assert!(text.contains("Quarterly report") && text.contains("Revenue rose 12%"), "{text}");
        assert!(read_bytes("bad.pdf", b"%PDF-1.4 nonsense", None).is_err());
    }

    #[test]
    fn word_documents_round_trip() {
        let doc = make_docx("# Trip plan\nDay 1: Paris & Lyon\n- pack <passport>\n\nDone").unwrap();
        let text = read_bytes("plan.docx", &doc, None).unwrap();
        assert_eq!(text, "Trip plan\nDay 1: Paris & Lyon\n• pack <passport>\n\nDone");
    }

    #[test]
    fn spreadsheets_round_trip_with_numbers() {
        let book = make_xlsx("Item,Cost\nCoffee,3.5\n\"Cake, lemon\",4\n").unwrap();
        let text = read_bytes("costs.xlsx", &book, None).unwrap();
        assert!(text.starts_with("Sheet \"Sheet1\" (3 rows × 2 columns):"), "{text}");
        assert!(text.contains("Cake, lemon | 4") && text.contains("Coffee | 3.5"), "{text}");
        assert_eq!(column(0), "A");
        assert_eq!(column(27), "AB");
    }

    #[test]
    fn csv_quotes_and_unknown_binaries() {
        assert_eq!(parse_csv("a,\"b, \"\"c\"\"\"\n1,2"), vec![vec!["a", "b, \"c\""], vec!["1", "2"]]);
        assert!(read_bytes("x.csv", b"name,age\nAna,40\n", None).unwrap().contains("Ana | 40"));
        assert!(read_bytes("app.exe", &[0x4d, 0x5a, 0, 0, 1], None).is_err());
        assert!(make_document("pptx", "x").is_err());
    }
}
