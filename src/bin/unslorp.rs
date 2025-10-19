use anyhow::{Context, Result, anyhow};
use lopdf::{Document, ObjectId};
use slorpit::{ArchiveCatalog, CATALOG_KEY};
use std::fs;
use std::io::Write;
use std::path::Path;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <archive.pdf> [output_directory]", args[0]);
        std::process::exit(1);
    }

    let input_path = &args[1];
    let output_dir = if args.len() >= 3 { &args[2] } else { "." };

    println!("Extracting PDF archive: {}", input_path);

    let doc = Document::load(input_path)
        .with_context(|| format!("Failed to load PDF from {}", input_path))?;

    let catalog_id = find_catalog_id(&doc)?;

    let catalog = extract_catalog(&doc, catalog_id)?;

    println!("Found {} files in archive", catalog.files.len());

    let output_path = Path::new(output_dir);
    fs::create_dir_all(output_path)?;

    let file_streams = find_file_streams(&doc)?;

    for (idx, file_entry) in catalog.files.iter().enumerate() {
        println!("  Extracting: {}", file_entry.path);

        if idx >= file_streams.len() {
            eprintln!("Warning: Missing stream for {}", file_entry.path);
            continue;
        }

        let stream_id = file_streams[idx];
        let content = extract_file_content(&doc, stream_id)?;

        let file_path = output_path.join(&file_entry.path);

        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = fs::File::create(&file_path)
            .with_context(|| format!("Failed to create {}", file_path.display()))?;
        file.write_all(&content)?;

        if let Some(modified) = file_entry.modified {
            let time = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(modified);
            let _ =
                filetime::set_file_mtime(&file_path, filetime::FileTime::from_system_time(time));
        }
    }

    println!(
        "Successfully extracted {} files to {}",
        catalog.files.len(),
        output_dir
    );

    Ok(())
}

fn find_catalog_id(doc: &Document) -> Result<ObjectId> {
    let trailer = &doc.trailer;
    let root_obj = trailer.get(b"Root").context("No Root in PDF trailer")?;
    let root_id = root_obj.as_reference().context("Root is not a reference")?;

    let root = doc
        .get_object(root_id)
        .context("Failed to get root object")?
        .as_dict()
        .context("Invalid Root catalog")?;

    let catalog_obj = root
        .get(CATALOG_KEY.as_bytes())
        .context("No Slorpit catalog found in PDF")?;
    let catalog_id = catalog_obj
        .as_reference()
        .context("Catalog is not a reference")?;

    Ok(catalog_id)
}

fn extract_catalog(doc: &Document, catalog_id: ObjectId) -> Result<ArchiveCatalog> {
    let catalog_obj = doc
        .get_object(catalog_id)
        .map_err(|e| anyhow!("Catalog object not found: {}", e))?;

    let stream = catalog_obj
        .as_stream()
        .map_err(|e| anyhow!("Catalog is not a stream: {}", e))?;

    let content = if stream.dict.get(b"Filter").is_ok() {
        stream.decompressed_content()?
    } else {
        stream.content.clone()
    };
    let catalog: ArchiveCatalog =
        serde_json::from_slice(&content).with_context(|| "Failed to parse catalog JSON")?;

    Ok(catalog)
}

fn find_file_streams(doc: &Document) -> Result<Vec<ObjectId>> {
    let mut file_streams = Vec::new();

    for (object_id, object) in &doc.objects {
        if let Ok(stream) = object.as_stream() {
            if let Ok(dict) = stream.dict.get(b"Type") {
                if let Ok(type_name) = dict.as_name() {
                    if type_name == b"EmbeddedFile" {
                        file_streams.push(*object_id);
                    }
                }
            }
        }
    }

    file_streams.sort_by_key(|(id, generation)| (*id, *generation));

    Ok(file_streams)
}

fn extract_file_content(doc: &Document, object_id: ObjectId) -> Result<Vec<u8>> {
    let object = doc
        .get_object(object_id)
        .map_err(|e| anyhow!("Object not found: {:?}: {}", object_id, e))?;

    let stream = object
        .as_stream()
        .map_err(|e| anyhow!("Object is not a stream: {:?}: {}", object_id, e))?;

    let content = stream
        .decompressed_content()
        .with_context(|| format!("Failed to decompress stream: {:?}", object_id))?;

    Ok(content)
}
