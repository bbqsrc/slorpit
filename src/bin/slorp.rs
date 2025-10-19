use anyhow::{Context, Result};
use lopdf::{Document, Object, Stream, dictionary};
use slorpit::{ArchiveCatalog, CATALOG_KEY, FileEntry};
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 3 {
        eprintln!("Usage: {} <output.pdf> <files...>", args[0]);
        std::process::exit(1);
    }

    let output_path = &args[1];
    let input_paths: Vec<&str> = args[2..].iter().map(|s| s.as_str()).collect();

    println!("Creating PDF archive: {}", output_path);

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut catalog = ArchiveCatalog::new();

    let mut file_objects = Vec::new();

    for input_path in &input_paths {
        let path = Path::new(input_path);

        if path.is_file() {
            process_file(&mut doc, &path, &path, &mut catalog, &mut file_objects)?;
        } else if path.is_dir() {
            for entry in WalkDir::new(path)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                process_file(
                    &mut doc,
                    entry.path(),
                    path,
                    &mut catalog,
                    &mut file_objects,
                )?;
            }
        } else {
            eprintln!(
                "Warning: {} is neither a file nor directory, skipping",
                input_path
            );
        }
    }

    let catalog_json = serde_json::to_string(&catalog)?;
    let catalog_stream = Stream::new(
        dictionary! {
            "Type" => "Metadata",
            "Subtype" => "SlorpitArchive",
        },
        catalog_json.into_bytes(),
    );
    let catalog_id = doc.add_object(catalog_stream);

    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Courier",
    });

    let page_content = create_file_listing_content(&catalog)?;
    let content_stream = Stream::new(dictionary! {}, page_content.as_bytes().to_vec());
    let content_id = doc.add_object(content_stream);

    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "Resources" => dictionary! {
            "Font" => dictionary! {
                "F1" => font_id,
            },
        },
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    });

    let pages = dictionary! {
        "Type" => "Pages",
        "Kids" => vec![page_id.into()],
        "Count" => 1,
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let catalog_dict = dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
        CATALOG_KEY => catalog_id,
    };
    let catalog_obj_id = doc.add_object(catalog_dict);
    doc.trailer.set("Root", catalog_obj_id);

    doc.compress();

    let save_options = lopdf::SaveOptions::builder()
        .compression_level(9)
        .use_object_streams(true)
        .max_objects_per_stream(200)
        .build();

    let mut file = fs::File::create(output_path)
        .with_context(|| format!("Failed to create output file {}", output_path))?;
    doc.save_with_options(&mut file, save_options)
        .with_context(|| format!("Failed to save PDF to {}", output_path))?;

    println!(
        "Successfully archived {} files to {}",
        catalog.files.len(),
        output_path
    );

    Ok(())
}

fn process_file(
    doc: &mut Document,
    file_path: &Path,
    base_path: &Path,
    catalog: &mut ArchiveCatalog,
    _file_objects: &mut Vec<(u32, u16)>,
) -> Result<()> {
    let mut relative_path = file_path
        .strip_prefix(base_path)
        .unwrap_or(file_path)
        .to_string_lossy()
        .to_string();

    if relative_path.is_empty() {
        relative_path = file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());
    }

    println!("  Adding: {}", relative_path);

    let content =
        fs::read(file_path).with_context(|| format!("Failed to read {}", file_path.display()))?;

    let metadata = fs::metadata(file_path)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());

    let compressed = compress_data(&content)?;

    let mut stream_dict = dictionary! {
        "Type" => "EmbeddedFile",
        "Length" => compressed.len() as i64,
        "Filter" => "FlateDecode",
    };

    stream_dict.set(
        "FileName",
        Object::String(
            relative_path.as_bytes().to_vec(),
            lopdf::StringFormat::Literal,
        ),
    );

    let stream = Stream::new(stream_dict, compressed);

    let _object_id = doc.add_object(stream);

    catalog.files.push(FileEntry {
        path: relative_path,
        size: content.len() as u64,
        modified,
    });

    Ok(())
}

fn create_file_listing_content(catalog: &ArchiveCatalog) -> Result<String> {
    let mut content = String::new();

    content.push_str("BT\n");
    content.push_str("/F1 12 Tf\n");
    content.push_str("50 750 Td\n");
    content.push_str("(SLORPIT PDF Archive) Tj\n");
    content.push_str("0 -20 Td\n");
    content.push_str("/F1 10 Tf\n");

    let header = format!("(Archive contains {} files)", catalog.files.len());
    content.push_str(&header);
    content.push_str(" Tj\n");
    content.push_str("0 -25 Td\n");

    content.push_str("/F1 9 Tf\n");
    content.push_str("(Filename) Tj\n");
    content.push_str("300 0 Td\n");
    content.push_str("(Size) Tj\n");
    content.push_str("100 0 Td\n");
    content.push_str("(Modified) Tj\n");
    content.push_str("-400 -15 Td\n");

    for file in &catalog.files {
        let filename = escape_pdf_string(&file.path);
        content.push_str(&format!("({}) Tj\n", filename));
        content.push_str("300 0 Td\n");

        let size_str = format_size(file.size);
        content.push_str(&format!("({}) Tj\n", size_str));
        content.push_str("100 0 Td\n");

        let modified_str = if let Some(ts) = file.modified {
            format_timestamp(ts)
        } else {
            "N/A".to_string()
        };
        content.push_str(&format!("({}) Tj\n", modified_str));

        content.push_str("-400 -12 Td\n");
    }

    content.push_str("ET\n");

    Ok(content)
}

fn escape_pdf_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
        .chars()
        .filter(|c| c.is_ascii() && !c.is_control())
        .collect()
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn format_timestamp(ts: u64) -> String {
    use chrono::DateTime;

    if let Some(datetime) = DateTime::from_timestamp(ts as i64, 0) {
        datetime.format("%Y-%m-%d %H:%M").to_string()
    } else {
        "N/A".to_string()
    }
}

fn compress_data(data: &[u8]) -> Result<Vec<u8>> {
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use std::io::Write;

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}
