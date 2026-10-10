//! File uploads for admin content forms.
//!
//! `axum::Form` cannot read a multipart body, so the notice/news/event handlers
//! take a `Multipart` extractor and go through [`read`] instead. Files are
//! written under `<upload_dir>/<folder>/` and recorded in the `uploads` table;
//! `/uploads` is already served by `ServeDir` in `main.rs`.

use std::{collections::HashMap, path::Path};

use argon2::password_hash::rand_core::{OsRng, RngCore};
use axum::extract::Multipart;
use sqlx::PgPool;

use crate::error::{internal, AppError};

/// Largest single file we accept. The route layer caps the whole body slightly higher.
pub const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;

/// Total bytes across every part of one form.
const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

/// Extensions we are willing to write, keyed by the browser's content type.
/// The extension is chosen from this table, never from the client's filename.
const IMAGE_TYPES: &[(&str, &str)] = &[
    ("image/jpeg", "jpg"),
    ("image/png", "png"),
    ("image/gif", "gif"),
    ("image/webp", "webp"),
];

const DOC_TYPES: &[(&str, &str)] = &[
    ("application/pdf", "pdf"),
    ("application/msword", "doc"),
    (
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "docx",
    ),
];

/// The extension allowed for this content type, or `None` when the type is not on the
/// allowlist. `images_only` rejects documents (used for the photo slots).
fn allowed_extension(mime: &str, images_only: bool) -> Option<&'static str> {
    let mime = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let image = IMAGE_TYPES.iter().find(|(m, _)| *m == mime).map(|(_, e)| *e);
    match image {
        Some(ext) => Some(ext),
        None if images_only => None,
        None => DOC_TYPES
            .iter()
            .find(|(m, _)| *m == mime)
            .map(|(_, e)| *e),
    }
}

/// One file part, keeping the browser's filename and content type for the uploads table.
pub struct IncomingFile {
    pub filename: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// One parsed multipart body: plain fields plus any file parts, keyed by field name.
///
/// `fields` keeps the first value under each name, which is all a normal form
/// needs. A table of repeated inputs (one per row) cannot be read that way, so
/// `values` holds every value in order; a urlencoded body cannot carry those at
/// all, which is why such a form has to post as multipart.
#[derive(Default)]
pub struct ParsedForm {
    pub fields: HashMap<String, String>,
    pub values: HashMap<String, Vec<String>>,
    pub files: HashMap<String, IncomingFile>,
}

impl ParsedForm {
    /// A text field, trimmed. Empty when absent.
    pub fn field(&self, name: &str) -> &str {
        self.field_opt(name).unwrap_or("")
    }

    /// The field only if the form actually sent it, so a caller can tell
    /// "posted empty" apart from "not posted at all". The second case must not
    /// overwrite stored data: a form that fails partway would otherwise blank
    /// every setting it did not manage to send.
    pub fn field_opt(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(|v| v.trim())
    }

    /// Every value submitted under `name`, in the order the rows appeared.
    pub fn repeated(&self, name: &str) -> &[String] {
        self.values.get(name).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn file(&self, name: &str) -> Option<&IncomingFile> {
        self.files.get(name)
    }

    /// A checkbox-style text field, where the box being ticked is the value we want.
    pub fn flag(&self, name: &str) -> bool {
        matches!(self.field(name), "on" | "true" | "1" | "yes")
    }
}

/// Read a multipart body into text fields and in-memory files.
pub async fn read(mut multipart: Multipart) -> Result<ParsedForm, AppError> {
    let mut form = ParsedForm::default();
    let mut total = 0usize;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| AppError::BadRequest("Could not read the uploaded form.".into()))?
    {
        let name = field.name().unwrap_or_default().to_string();

        // No filename means this part is an ordinary text field.
        let Some(filename) = field.file_name().map(str::to_owned) else {
            let text = field
                .text()
                .await
                .map_err(|_| AppError::BadRequest("Could not read the uploaded form.".into()))?;
            total += text.len();
            if total > MAX_BODY_BYTES {
                return Err(AppError::BadRequest("That form is too large.".into()));
            }
            form.values.entry(name.clone()).or_default().push(text.clone());
            form.fields.entry(name).or_insert(text);
            continue;
        };

        // An empty filename is the browser's way of saying "no file chosen".
        if filename.trim().is_empty() {
            continue;
        }

        let mime = field.content_type().unwrap_or_default().to_string();
        let bytes = field
            .bytes()
            .await
            .map_err(|_| AppError::BadRequest(format!("Could not read \"{filename}\".")))?;

        if bytes.len() > MAX_FILE_BYTES {
            return Err(AppError::BadRequest(format!(
                "\"{filename}\" is larger than the {} MB limit.",
                MAX_FILE_BYTES / 1024 / 1024
            )));
        }
        total += bytes.len();
        if total > MAX_BODY_BYTES {
            return Err(AppError::BadRequest("That form is too large.".into()));
        }

        // A repeated field name keeps the first file, so a crafted form cannot
        // silently replace an image with something else.
        form.files.entry(name).or_insert(IncomingFile {
            filename,
            mime,
            bytes: bytes.to_vec(),
        });
    }

    Ok(form)
}

/// The client's filename, stripped of any directory part, for display in the uploads table.
fn display_name(original: &str) -> String {
    Path::new(original)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty() && n != "." && n != "..")
        .unwrap_or_else(|| "upload".to_string())
}

/// Write one uploaded file into `<upload_dir>/<folder>/` and record it in `uploads`.
///
/// Returns the public web path (`/uploads/<folder>/<name>`) to store on the content row.
pub async fn save(
    db: &PgPool,
    upload_dir: &str,
    folder: &str,
    file: &IncomingFile,
    images_only: bool,
    uploaded_by: Option<i64>,
) -> Result<String, AppError> {
    let label = display_name(&file.filename);
    if file.bytes.is_empty() {
        return Err(AppError::BadRequest(format!("\"{label}\" is empty.")));
    }
    let Some(ext) = allowed_extension(&file.mime, images_only) else {
        return Err(AppError::BadRequest(format!(
            "\"{label}\" is not an accepted file type. Use a JPG, PNG, GIF or WebP{}.",
            if images_only { "" } else { ", or a PDF" }
        )));
    };

    // Never trust the incoming name for the path: generate our own and keep the
    // original only as a label.
    let mut suffix = [0u8; 8];
    OsRng.fill_bytes(&mut suffix);
    let name = format!(
        "{}_{}.{ext}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default(),
        suffix.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );

    let dir = Path::new(upload_dir).join(folder);
    std::fs::create_dir_all(&dir).map_err(|e| {
        tracing::error!(error = %e, "could not create the upload directory");
        AppError::Internal("Could not save the file.".into())
    })?;
    std::fs::write(dir.join(&name), &file.bytes).map_err(|e| {
        tracing::error!(error = %e, "could not write the uploaded file");
        AppError::Internal("Could not save the file.".into())
    })?;

    let web_path = format!("/uploads/{folder}/{name}");
    if let Err(error) = sqlx::query(
        r#"INSERT INTO uploads (path, original_name, mime_type, size_bytes, uploaded_by)
           VALUES ($1, $2, $3, $4, $5)"#,
    )
    .bind(&web_path)
    .bind(&label)
    .bind(&file.mime)
    .bind(file.bytes.len() as i64)
    .bind(uploaded_by)
    .execute(db)
    .await
    {
        let _ = std::fs::remove_file(dir.join(&name));
        return Err(internal(error));
    }

    Ok(web_path)
}

/// Save the file in `field`, or keep the path already on the row when none was posted.
///
/// `keep` is what is currently stored on the content row; posting a replacement is the
/// only way to change it, so an empty file input never wipes an existing image.
pub async fn save_optional(
    db: &PgPool,
    upload_dir: &str,
    folder: &str,
    form: &ParsedForm,
    field: &str,
    keep: &str,
    images_only: bool,
    uploaded_by: Option<i64>,
) -> Result<String, AppError> {
    match form.file(field) {
        Some(file) => save(db, upload_dir, folder, file, images_only, uploaded_by).await,
        None => Ok(keep.to_string()),
    }
}
