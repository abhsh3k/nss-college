//! File-based student import.
//!
//! An uploaded CSV or Excel list is parsed into [`ParsedRow`]s and written to the
//! staging tables (`import_batches` / `import_rows`). Nothing reaches `students`
//! or `users` until the admin has reviewed the staged rows and commits them.
//!
//! Parsing is deliberately forgiving about column names and delimiters because
//! the lists come from spreadsheets, but strict about what a row must contain:
//! anything missing is reported against the row rather than failing the upload.

use std::io::Cursor;

use calamine::{open_workbook_auto_from_rs, Data, Reader};
use sqlx::{FromRow, PgPool};

type Res<T> = Result<T, sqlx::Error>;

/// Accounts are created one at a time and each password is hashed with Argon2,
/// which is deliberately slow, so a single upload is capped.
pub const MAX_ROWS: usize = 500;

/// One row of an uploaded file, before it is staged.
#[derive(Debug, Clone)]
pub struct ParsedRow {
    pub admission_no: String,
    pub name: String,
    pub email: String,
    pub phone: String,
    /// Free text, matched against programmes at review time.
    pub programme: String,
    pub semester: String,
    pub year: String,
    /// Set when the line itself could not be read. The row is still staged so
    /// the admin can fix it by hand instead of editing the file again.
    pub error: String,
}

//// The outcome of parsing a file: the rows, plus how many were left out at the cap.
#[derive(Debug)]
pub struct Parsed {
    pub rows: Vec<ParsedRow>,
    pub dropped: usize,
}

// ---------- Reading the file ----------

/// Parses an uploaded CSV or Excel file into rows.
///
/// The format is taken from the extension, not from the content, and only the
/// formats listed below are accepted.
pub fn parse(bytes: &[u8], filename: &str) -> Result<Parsed, String> {
    if bytes.is_empty() {
        return Err("That file is empty.".into());
    }
    let ext = filename.rsplit('.').next().unwrap_or_default().trim().to_lowercase();
    let table = match ext.as_str() {
        "csv" | "tsv" | "txt" => delimited(bytes)?,
        "xlsx" | "xlsm" | "xls" | "ods" => spreadsheet(bytes)?,
        "" => return Err("That file has no name, so its type could not be worked out.".into()),
        other => {
            return Err(format!(
                "'{other}' files cannot be imported. Upload a .csv, .tsv, .xlsx or .xls file."
            ))
        }
    };
    rows_from(&table)
}

/// Tab-separated when the first line looks like a table copied out of Excel,
/// comma-separated otherwise.
fn delimited(bytes: &[u8]) -> Result<Vec<Vec<String>>, String> {
    let text = decode(bytes)?;
    let delimiter = match text.lines().next().unwrap_or_default().split_once('\t') {
        Some(_) => b'\t',
        None => b',',
    };
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .trim(csv::Trim::All)
        // The header is data here: it has to come through as row one so the
        // column names can be matched rather than trusted.
        .has_headers(false)
        .from_reader(text.as_bytes());

    let mut table = Vec::new();
    for record in reader.records() {
        // An unreadable line still occupies its place in the table so the line
        // numbers shown on the review page stay true to the file.
        match record {
            Ok(r) => table.push(r.iter().map(str::to_string).collect()),
            Err(_) => table.push(Vec::new()),
        }
    }
    if table.is_empty() {
        return Err("That file has no rows.".into());
    }
    Ok(table)
}

/// The first sheet of an Excel or OpenDocument workbook.
fn spreadsheet(bytes: &[u8]) -> Result<Vec<Vec<String>>, String> {
    let mut book = open_workbook_auto_from_rs(Cursor::new(bytes.to_vec()))
        .map_err(|e| format!("That workbook could not be opened: {e}"))?;
    let sheet = book
        .worksheet_range_at(0)
        .ok_or_else(|| "That workbook has no sheets.".to_string())?;
    let range = sheet.map_err(|e| format!("That workbook could not be read: {e}"))?;
    Ok(range.rows().map(|row| row.iter().map(cell_text).collect()).collect())
}

/// Spreadsheet cells arrive typed; render them the way the header row expects.
fn cell_text(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::Error(e) => e.to_string(),
        // Excel stores whole numbers as floats, so 3.0 should read as "3".
        Data::Float(f) if f.fract() == 0.0 => format!("{}", *f as i64),
        other => other.to_string(),
    }
}

/// UTF-8 with or without a BOM, plus the UTF-16 files Excel writes for "CSV".
fn decode(bytes: &[u8]) -> Result<String, String> {
    let bad = || "That file is not readable as text. Re-save it as CSV (UTF-8).".to_string();
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        let big = bytes[1] == 0xFE;
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| if big { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) })
            .collect();
        return String::from_utf16(&units).map_err(|_| bad());
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8(bytes.to_vec()).map_err(|_| bad())
}

// ---------- Header names ----------

/// Lower case, with punctuation flattened to single underscores, so that
/// "Admission No.", "admission-no" and "ADMISSION_NO" are all the same column.
fn norm_header(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut gap = false;
    for ch in raw.trim().chars() {
        if ch.is_alphanumeric() {
            if gap && !out.is_empty() {
                out.push('_');
            }
            gap = false;
            out.extend(ch.to_lowercase());
        } else {
            gap = true;
        }
    }
    out
}

const ADMISSION: &[&str] = &[
    "admission_no",
    "admission",
    "admission_number",
    "admissionnumber",
    "adm_no",
    "admno",
    "reg_no",
    "regno",
    "register_no",
    "roll_no",
    "rollno",
    "usn",
];
const NAME: &[&str] = &["name", "student_name", "full_name", "fullname", "student"];
const EMAIL: &[&str] = &["email", "email_address", "emailaddress", "e_mail", "mail"];
const PHONE: &[&str] = &["phone", "phone_no", "phoneno", "mobile", "mobile_no", "contact"];
const PROGRAMME: &[&str] = &[
    "programme",
    "program",
    "programme_name",
    "programme_code",
    "program_name",
];
const SEMESTER: &[&str] = &["semester", "sem", "semester_no", "semester_number"];
const YEAR: &[&str] = &[
    "batch_year",
    "batchyear",
    "batch",
    "admission_year",
    "admissionyear",
    "year",
    "year_of_admission",
    "joining_year",
];

fn column(headers: &[String], aliases: &[&str]) -> Option<usize> {
    aliases.iter().find_map(|a| headers.iter().position(|h| h == a))
}

fn cell_text_at(row: &[String], index: Option<usize>) -> String {
    index.and_then(|i| row.get(i)).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn rows_from(table: &[Vec<String>]) -> Result<Parsed, String> {
    let headers: Vec<String> = table.first().map(|r| r.iter().map(|h| norm_header(h)).collect()).unwrap_or_default();
    let Some(c_adm) = column(&headers, ADMISSION) else {
        return Err("The first line must have a column called admission_no.".into());
    };
    let Some(c_name) = column(&headers, NAME) else {
        return Err("The first line must have a column called name.".into());
    };
    let (c_email, c_phone) = (
        column(&headers, EMAIL),
        column(&headers, PHONE),
    );
    let (c_programme, c_semester, c_year) = (
        column(&headers, PROGRAMME),
        column(&headers, SEMESTER),
        column(&headers, YEAR),
    );

    let data = &table[1..];
    let dropped = data.len().saturating_sub(MAX_ROWS);
    let mut rows = Vec::with_capacity(data.len().min(MAX_ROWS));

    for raw in data.iter().take(MAX_ROWS) {
        let mut row = ParsedRow {
            admission_no: cell_text_at(raw, Some(c_adm)),
            name: cell_text_at(raw, Some(c_name)),
            email: cell_text_at(raw, c_email),
            phone: cell_text_at(raw, c_phone),
            programme: cell_text_at(raw, c_programme),
            semester: cell_text_at(raw, c_semester),
            year: cell_text_at(raw, c_year),
            error: String::new(),
        };
        // Blank lines are common at the end of exported files.
        if raw.is_empty() || (row.admission_no.is_empty() && row.name.is_empty()) {
            continue;
        }
        if row.admission_no.is_empty() {
            row.error = "No admission number.".into();
        } else if row.name.is_empty() {
            row.error = "No name.".into();
        }
        rows.push(row);
    }

    if rows.is_empty() {
        return Err("That file has a header row but no students in it.".into());
    }
    Ok(Parsed { rows, dropped })
}

// ---------- Staged batches ----------

/// One upload: the file it came from plus the defaults for its rows.
#[derive(Debug, FromRow)]
pub struct Batch {
    pub id: i64,
    pub source_name: String,
    pub programme_name: String,
    pub semester: i32,
    pub batch_year: i32,
}

const BATCH_SQL: &str = r#"SELECT b.id, b.source_name, p.name AS programme_name,
          b.semester, b.batch_year
   FROM import_batches b JOIN programmes p ON p.id = b.programme_id"#;

pub async fn latest_batch(db: &PgPool) -> Res<Option<Batch>> {
    sqlx::query_as::<_, Batch>(&format!("{BATCH_SQL} ORDER BY b.id DESC LIMIT 1"))
        .fetch_optional(db)
        .await
}

pub async fn batch(db: &PgPool, id: i64) -> Res<Option<Batch>> {
    sqlx::query_as::<_, Batch>(&format!("{BATCH_SQL} WHERE b.id = $1"))
        .bind(id)
        .fetch_optional(db)
        .await
}

/// Replaces any previous batch: an admin only ever has one import on the go.
pub async fn stage(
    db: &PgPool,
    source_name: &str,
    programme_id: i64,
    semester: i32,
    batch_year: i32,
    created_by: Option<i64>,
    rows: &[ParsedRow],
) -> Res<i64> {
    let mut tx = db.begin().await?;
    sqlx::query("DELETE FROM import_batches")
        .execute(&mut *tx)
        .await?;
    let batch_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO import_batches (source_name, programme_id, semester, batch_year, created_by)
           VALUES ($1, $2, $3, $4, $5) RETURNING id"#,
    )
    .bind(source_name)
    .bind(programme_id)
    .bind(semester)
    .bind(batch_year)
    .bind(created_by)
    .fetch_one(&mut *tx)
    .await?;

    for (i, row) in rows.iter().enumerate() {
        sqlx::query(
            r#"INSERT INTO import_rows
                 (batch_id, line_no, admission_no, name, email, phone,
                  programme_text, semester_text, year_text, note)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"#,
        )
        .bind(batch_id)
        .bind(i as i32 + 2)
        .bind(&row.admission_no)
        .bind(&row.name)
        .bind(&row.email)
        .bind(&row.phone)
        .bind(&row.programme)
        .bind(&row.semester)
        .bind(&row.year)
        .bind(&row.error)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(batch_id)
}

pub async fn discard(db: &PgPool, batch_id: i64) -> Res<()> {
    sqlx::query("DELETE FROM import_batches WHERE id = $1")
        .bind(batch_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Removes batches old enough that their one-time passwords are long gone.
pub async fn purge_stale(db: &PgPool) -> Res<()> {
    sqlx::query("SELECT purge_stale_import_batches()")
        .execute(db)
        .await?;
    Ok(())
}

// ---------- Staged rows ----------

/// A row on the review page, with its values already resolved against the batch.
#[derive(Debug, FromRow)]
pub struct StagedRow {
    pub id: i64,
    pub line_no: i32,
    pub admission_no: String,
    pub name: String,
    pub email: String,
    pub phone: String,
    pub programme_text: String,
    pub semester_text: String,
    pub year_text: String,
    pub include: bool,
    pub status: String,
    pub note: String,
    pub batch_programme_id: i64,
    pub batch_programme_name: String,
    pub batch_semester: i32,
    pub batch_year: i32,
    pub resolved_programme_id: Option<i64>,
    pub resolved_programme_name: Option<String>,
}

/// Programme names, offered as a list on the review page so a cell that
/// overrides the batch default is easier to type correctly.
pub async fn programme_names(db: &PgPool) -> Res<Vec<String>> {
    sqlx::query_scalar("SELECT name FROM programmes ORDER BY name")
        .fetch_all(db)
        .await
}

const ROWS_SQL: &str = r#"SELECT r.id, r.line_no, r.admission_no, r.name, r.email, r.phone,
          r.programme_text, r.semester_text, r.year_text, r.include, r.status, r.note,
          b.programme_id AS batch_programme_id, p.name AS batch_programme_name,
          b.semester AS batch_semester, b.batch_year AS batch_year,
          prog.id AS resolved_programme_id, prog.name AS resolved_programme_name
   FROM import_rows r
   JOIN import_batches b ON b.id = r.batch_id
   JOIN programmes p ON p.id = b.programme_id
   LEFT JOIN programmes prog
          ON lower(prog.name) = lower(btrim(r.programme_text))
          OR lower(prog.slug) = lower(btrim(r.programme_text))
   WHERE r.batch_id = $1
   ORDER BY r.line_no"#;

pub async fn rows(db: &PgPool, batch_id: i64) -> Res<Vec<StagedRow>> {
    sqlx::query_as::<_, StagedRow>(ROWS_SQL).bind(batch_id).fetch_all(db).await
}

impl StagedRow {
    pub fn is_created(&self) -> bool {
        self.status == "created"
    }

    /// The programme this row ends up in: its own, or the batch default.
    pub fn programme_id(&self) -> Option<i64> {
        if self.programme_text.trim().is_empty() {
            Some(self.batch_programme_id)
        } else {
            self.resolved_programme_id
        }
    }

    pub fn programme_name(&self) -> &str {
        match self.programme_id() {
            Some(id) if id == self.batch_programme_id => &self.batch_programme_name,
            Some(_) => self.resolved_programme_name.as_deref().unwrap_or("unknown programme"),
            None => "unknown programme",
        }
    }

    pub fn semester(&self) -> Option<i32> {
        let text = self.semester_text.trim();
        match text.parse::<i32>() {
            Ok(v) if (1..=12).contains(&v) => Some(v),
            Ok(_) => None,
            Err(_) if text.is_empty() => Some(self.batch_semester),
            Err(_) => None,
        }
    }

    pub fn year(&self) -> Option<i32> {
        let text = self.year_text.trim();
        match text.parse::<i32>() {
            Ok(v) if (2000..=2100).contains(&v) => Some(v),
            Ok(_) => None,
            Err(_) if text.is_empty() => Some(self.batch_year),
            Err(_) => None,
        }
    }

    /// Everything wrong with this row, in the order the admin should fix it.
    pub fn problems(&self) -> Vec<String> {
        if self.is_created() {
            return Vec::new();
        }
        let mut out = Vec::new();
        if self.admission_no.trim().is_empty() {
            out.push("Admission number is required.".into());
        }
        if self.name.trim().is_empty() {
            out.push("Name is required.".into());
        }
        let email = self.email.trim();
        if !email.is_empty() && !email.contains('@') {
            out.push("That email address is not valid.".into());
        }
        match self.programme_id() {
            Some(_) => {}
            None => out.push(format!("No programme called \"{}\".", self.programme_text.trim())),
        }
        if self.semester().is_none() {
            out.push("Semester must be a number from 1 to 12.".into());
        }
        if self.year().is_none() {
            out.push("Admission year must be between 2000 and 2100.".into());
        }
        out
    }

    /// Whether this row will be created when the batch is committed.
    pub fn ready(&self) -> bool {
        self.include && !self.is_created() && self.problems().is_empty()
    }
}



/// The values the admin typed on the review page.
pub struct RowEdit {
    pub id: i64,
    pub admission_no: String,
    pub name: String,
    pub email: String,
    pub phone: String,
    pub programme_text: String,
    pub semester_text: String,
    pub year_text: String,
    pub include: bool,
}

pub async fn save_rows(db: &PgPool, batch_id: i64, edits: &[RowEdit]) -> Res<()> {
    let mut tx = db.begin().await?;
    for e in edits {
        sqlx::query(
            r#"UPDATE import_rows
               SET admission_no = $3, name = $4, email = $5, phone = $6,
                   programme_text = $7, semester_text = $8, year_text = $9, include = $10
               WHERE id = $1 AND batch_id = $2"#,
        )
        .bind(e.id)
        .bind(batch_id)
        .bind(e.admission_no.trim())
        .bind(e.name.trim())
        .bind(e.email.trim())
        .bind(e.phone.trim())
        .bind(e.programme_text.trim())
        .bind(e.semester_text.trim())
        .bind(e.year_text.trim())
        .bind(e.include)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Appends a blank row so a short file can be topped up by hand.
pub async fn add_row(db: &PgPool, batch_id: i64) -> Res<()> {
    let next: i32 = sqlx::query_scalar("SELECT COALESCE(max(line_no), 1) + 1 FROM import_rows WHERE batch_id = $1")
        .bind(batch_id)
        .fetch_one(db)
        .await?;
    sqlx::query(
        "INSERT INTO import_rows (batch_id, line_no, note) VALUES ($1, $2, 'Added by hand.')",
    )
    .bind(batch_id)
    .bind(next)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn delete_row(db: &PgPool, batch_id: i64, row_id: i64) -> Res<()> {
    sqlx::query("DELETE FROM import_rows WHERE id = $1 AND batch_id = $2")
        .bind(row_id)
        .bind(batch_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Ticks every row, clears them all, or picks out the ones with nothing wrong.
pub async fn set_all(db: &PgPool, batch_id: i64, how: &str) -> Res<()> {
    let sql = match how {
        "none" => "UPDATE import_rows SET include = false WHERE batch_id = $1",
        "valid" => {
            r#"UPDATE import_rows SET include = true
               WHERE batch_id = $1 AND admission_no <> '' AND name <> ''
                 AND status <> 'created'"#
        }
        _ => "UPDATE import_rows SET include = true WHERE batch_id = $1",
    };
    sqlx::query(sql).bind(batch_id).execute(db).await?;
    Ok(())
}

/// Records the result of committing one row.
pub async fn mark(
    db: &PgPool,
    row_id: i64,
    status: &str,
    note: &str,
    temp_password: &str,
    user_id: Option<i64>,
) -> Res<()> {
    sqlx::query(
        "UPDATE import_rows SET status = $2, note = $3, temp_password = $4, user_id = $5 WHERE id = $1",
    )
    .bind(row_id)
    .bind(status)
    .bind(note)
    .bind(temp_password)
    .bind(user_id)
    .execute(db)
    .await?;
    Ok(())
}

/// The one-time passwords of a committed batch, for the results screen and CSV.
pub async fn credentials(db: &PgPool, batch_id: i64) -> Res<Vec<CredentialRow>> {
    sqlx::query_as::<_, CredentialRow>(
        r#"SELECT name AS name, admission_no AS login, temp_password AS password, note
           FROM import_rows
           WHERE batch_id = $1 AND status = 'created' AND temp_password <> ''
           ORDER BY line_no"#,
    )
    .bind(batch_id)
    .fetch_all(db)
    .await
}



/// One line of the one-time credentials table.
#[derive(Debug, FromRow)]
pub struct CredentialRow {
    pub name: String,
    pub login: String,
    pub password: String,
    pub note: String,
}

/// The credentials table as CSV, for handing the passwords over.
pub fn credentials_csv(rows: &[CredentialRow]) -> Result<Vec<u8>, csv::Error> {
    let mut writer = csv::Writer::from_writer(Vec::new());
    writer.write_record(["name", "admission_no", "temporary_password", "note"])?;
    for r in rows {
        writer.write_record([&r.name, &r.login, &r.password, &r.note])?;
    }
    let bytes = writer.into_inner().map_err(|e| csv::Error::from(e.into_error()))?;
    Ok(bytes)
}

/// A free email address in the placeholder domain, for students who have none.
pub fn placeholder_email(admission_no: &str) -> String {
    let stem: String = admission_no
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '_' || *c == '-')
        .collect::<String>()
        .to_lowercase();
    if stem.is_empty() {
        "student@students.college.local".to_string()
    } else {
        format!("{stem}@students.college.local")
    }
}
#[cfg(test)]
mod tests {
    use super::*;



    #[test]
    fn header_names_are_matched_loosely() {
        assert_eq!(norm_header("Admission No."), "admission_no");
        assert_eq!(norm_header("ADMISSION-NO"), "admission_no");
        assert_eq!(norm_header("AdmissionNo"), "admissionno");
        assert_eq!(norm_header(" Student Name "), "student_name");
        assert_eq!(norm_header("Year of Admission"), "year_of_admission");
    }

    #[test]
    fn reads_a_csv_list() {
        let csv = "admission_no,name,email,phone\n\
                   2501,Asha K,,9876500000\n\
                   2502,Rahul M,rahul@example.com,\n";
        let parsed = parse(csv.as_bytes(), "list.csv").unwrap();
        assert_eq!(parsed.dropped, 0);
        assert_eq!(parsed.rows.len(), 2);
        assert_eq!(parsed.rows[0].admission_no, "2501");
        assert_eq!(parsed.rows[0].name, "Asha K");
        assert_eq!(parsed.rows[0].phone, "9876500000");
        assert!(parsed.rows.iter().all(|r| r.error.is_empty()));
    }

    #[test]
    fn reads_per_row_programmes() {
        let csv = "Admission No.,Full Name,Programme,Semester,Year of Admission\n\
                   2501,Asha K,B.Sc. Electronics,5,2024\n";
        let parsed = parse(csv.as_bytes(), "list.csv").unwrap();
        let row = &parsed.rows[0];
        assert_eq!(row.programme, "B.Sc. Electronics");
        assert_eq!(row.semester, "5");
        assert_eq!(row.year, "2024");
    }

    #[test]
    fn reads_a_tab_separated_list_copied_from_a_sheet() {
        let tsv = "admission_no\tname\n2501\tAsha K\n";
        assert_eq!(parse(tsv.as_bytes(), "list.tsv").unwrap().rows.len(), 1);
    }

    #[test]
    fn a_row_missing_a_name_is_reported_not_dropped() {
        let csv = "admission_no,name\n2501,Asha K\n2502,\n";
        let parsed = parse(csv.as_bytes(), "list.csv").unwrap();
        assert_eq!(parsed.rows.len(), 2);
        assert_eq!(parsed.rows[1].error, "No name.");
    }

    #[test]
    fn blank_trailing_lines_are_ignored() {
        let csv = "admission_no,name\n2501,Asha K\n\n\n";
        assert_eq!(parse(csv.as_bytes(), "list.csv").unwrap().rows.len(), 1);
    }

    #[test]
    fn a_missing_required_column_is_an_error() {
        let csv = "roll,name\n2501,Asha K\n";
        let err = parse(csv.as_bytes(), "list.csv").unwrap_err();
        assert!(err.contains("admission_no"), "{err}");
    }

    #[test]
    fn an_unsupported_type_is_rejected() {
        let err = parse(b"%PDF-1.4", "list.pdf").unwrap_err();
        assert!(err.contains(".xlsx"), "{err}");
    }

    #[test]
    fn placeholder_emails_are_derived_from_the_admission_number() {
        assert_eq!(placeholder_email("BCA 25/01"), "bca2501@students.college.local");
        assert_eq!(placeholder_email("!!"), "student@students.college.local");
    }
}
