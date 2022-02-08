use reqwest::{
    header::{CONTENT_LENGTH, CONTENT_TYPE},
    Client,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{io::stdin, path::Path};
use time::{macros::format_description, OffsetDateTime};
use tokio::{
    fs,
    io::{self, AsyncReadExt, AsyncWriteExt},
    join, task,
};
use url::Url;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Email that invoices should be sent to.
// static INVOICE_EMAIL: &str = "Accounting <accounting@mobilecoin.com>";
static INVOICE_EMAIL: &str = "Diana <DianaNites@gmail.com>";

static APP_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"),);

/// Oauth Client ID
static CLIENT_ID: &str = env!("CLIENT_ID");
/// Oauth Client Secret
static CLIENT_SECRET: &str = env!("CLIENT_SECRET");
/// Oauth auth URL
static AUTH_URI: &str = env!("AUTH_URI");
/// Oauth token URL
static TOKEN_URI: &str = env!("TOKEN_URI");

/// List files on google drive
///
/// https://developers.google.com/drive/api/v3/reference/files/list
static FILE_LIST: &str = "https://www.googleapis.com/drive/v3/files";

/// Base spreadsheet URL
///
/// https://developers.google.com/sheets/api/reference/rest
static SPREADSHEET_BASE: &str = "https://sheets.googleapis.com/v4/spreadsheets";

/// Send an email as the authenticated user
static GMAIL_SEND: &str = "https://gmail.googleapis.com/upload/gmail/v1/users/me/messages/send";

/// Scopes our tokens need
static DRIVE_SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/drive",
    "https://www.googleapis.com/auth/gmail.send",
];

/// Oauth2 token information
#[derive(Debug, Serialize, Deserialize)]
struct Access {
    /// Temporary access token
    access_token: String,

    /// Seconds until `access_token` expires
    expires_in: u64,

    /// Token to refresh `access_token`
    // This isnt returned when refreshing
    #[serde(default)]
    refresh_token: String,

    /// Space separated list of scopes we got access to
    scope: String,

    /// Always Bearer
    token_type: String,
}

/// Google Drive File Resource
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileResource {
    /// File ID
    id: String,

    /// File Name
    name: String,

    /// File Mime type
    mime_type: String,

    /// Parent folder IDs
    parents: Vec<String>,

    /// Web link
    web_view_link: String,
}

/// Returned from Files::list
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListResponse {
    files: Vec<FileResource>,
}

/// Google Drive About resource
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriveAboutResponse {
    user: DriveUser,
}

/// Google Drive About::user resource
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriveUser {
    /// Users Display name
    display_name: String,

    /// Users email address
    email_address: String,
}

async fn check_access(client: &Client, path: &Path) -> Result<Access> {
    Ok(if path.exists() {
        let mut buf = io::BufReader::new(fs::File::open(path).await?);
        let mut json = Vec::new();
        buf.read_to_end(&mut json).await?;
        serde_json::from_slice(&json)?
    } else {
        first_access(client, path).await?
    })
}

/// Save oauth tokens
async fn save_access(access: Access, path: &Path) -> Result<Access> {
    let mut buf = io::BufWriter::new(fs::File::create(path).await?);
    let json = serde_json::to_vec_pretty(&access)?;
    buf.write_all(&json).await?;
    Ok(access)
}

/// First oauth access flow
async fn first_access(client: &Client, path: &Path) -> Result<Access> {
    let auth_url = Url::parse_with_params(
        AUTH_URI,
        &[
            //
            ("client_id", CLIENT_ID),
            ("redirect_uri", "urn:ietf:wg:oauth:2.0:oob"),
            ("response_type", "code"),
            ("scope", &DRIVE_SCOPES.join(" ")),
        ],
    )?;
    println!("Please open the following link: \n{}", auth_url);
    println!("Please copy the authorization code here:\n");
    let auth = task::spawn_blocking(|| {
        let mut auth = String::new();
        stdin().read_line(&mut auth).unwrap();
        auth
    })
    .await?;
    let token_url = Url::parse_with_params(
        TOKEN_URI,
        &[
            //
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
            ("code", &auth),
            ("code_verifier", ""),
            ("grant_type", "authorization_code"),
            ("redirect_uri", "urn:ietf:wg:oauth:2.0:oob"),
        ],
    )?;
    let res = client
        .post(token_url)
        .body("")
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(CONTENT_LENGTH, "0")
        .send()
        .await?;
    let text: Access = res.json().await?;
    if text.scope.split(' ').count() != DRIVE_SCOPES.len() {
        return Err("Required scopes not provided. Please select all scopes.".into());
    }
    save_access(text, path).await
}

/// Refresh our oauth token
async fn refresh(client: &Client, access: Access, path: &Path) -> Result<Access> {
    let token_url = Url::parse_with_params(
        TOKEN_URI,
        &[
            //
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
            ("grant_type", "refresh_token"),
            ("refresh_token", &access.refresh_token),
        ],
    )?;
    let res = client
        .post(token_url)
        .body("")
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(CONTENT_LENGTH, "0")
        .send()
        .await?;
    let text: Access = res.json().await?;
    save_access(
        Access {
            refresh_token: access.refresh_token,
            ..text
        },
        path,
    )
    .await
}

/// Get the Invoice Template and Output Folder
async fn get_files(client: &Client, access: &Access) -> Result<(FileResource, FileResource)> {
    let template = Url::parse_with_params(
        FILE_LIST,
        &[
            (
                "q",
                "name='Invoice Template' and mimeType='application/vnd.google-apps.spreadsheet' and trashed = false",
            ),
            ("fields", "files(id, name, mimeType, parents, webViewLink)"),
        ],
    )?;
    let folder = Url::parse_with_params(
        FILE_LIST,
        &[
            (
                "q",
                "mimeType='application/vnd.google-apps.folder' and name='Test' and trashed = false",
            ),
            ("fields", "files(id, name, mimeType, parents, webViewLink)"),
        ],
    )?;
    let (template, folder) = join!(
        client
            .get(template)
            .bearer_auth(&access.access_token)
            .send(),
        client.get(folder).bearer_auth(&access.access_token).send()
    );
    let (template, folder) = join!(
        template?.error_for_status()?.json::<ListResponse>(),
        folder?.error_for_status()?.json::<ListResponse>(),
    );
    let (mut template, mut folder) = (template?, folder?);
    let (template, folder) = (template.files.swap_remove(0), folder.files.swap_remove(0));
    Ok((template, folder))
}

/// Copy invoice template to final destination
async fn file_copy(
    client: &Client,
    access: &Access,
    folder_id: &str,
    file_id: &str,
    iso_time: &str,
) -> Result<FileResource> {
    let url = Url::parse_with_params(
        &format!("{}/{}/copy", FILE_LIST, file_id),
        &[("fields", "id, name, mimeType, parents, webViewLink")],
    )?;
    let res = client
        .post(url)
        .bearer_auth(&access.access_token)
        .json(&json!({
            "name": format!("Invoice-{}", iso_time),
            "parents": [folder_id]
        }))
        .send()
        .await?;
    let json = res.json::<FileResource>().await?;
    Ok(json)
}

/// Export invoice to PDF
async fn file_export(client: &Client, access: &Access, file_id: &str) -> Result<Vec<u8>> {
    let url = Url::parse_with_params(
        &format!("{}/{}/export", FILE_LIST, file_id),
        &[("mimeType", "application/pdf")],
    )?;
    let res = client
        .get(url)
        .bearer_auth(&access.access_token)
        .send()
        .await?;
    let json = res.bytes().await?;
    Ok(json.to_vec())
}

/// Get users name and email in the `Name <email>` format.
async fn get_email(client: &Client, access: &Access) -> Result<String> {
    let url = Url::parse_with_params(
        "https://www.googleapis.com/drive/v3/about",
        &[("fields", "user(displayName, emailAddress)")],
    )?;
    let res = client
        .get(url)
        .bearer_auth(&access.access_token)
        .send()
        .await?
        .json::<DriveAboutResponse>()
        .await?;
    Ok(format!(
        "{} <{}>",
        res.user.display_name, res.user.email_address
    ))
}

/// Ready the invoice for submission
///
/// This entails:
///
/// - Copying the template
/// - Updating the date
/// - Exporting as PDF
/// - Returning the PDF bytes
async fn ready_invoice(
    client: &Client,
    access: &Access,
    file_id: &str,
    folder_id: &str,
    sheets_time: &str,
    iso_time: &str,
    output_base: &Path,
) -> Result<Vec<u8>> {
    let file = file_copy(client, access, folder_id, file_id, iso_time).await?;
    let url = Url::parse_with_params(
        &format!("{}/{}/values/D9:E9", SPREADSHEET_BASE, file.id),
        &[("valueInputOption", "USER_ENTERED")],
    )?;
    client
        .put(url)
        .json(&json!({ "values": [[sheets_time]] }))
        .bearer_auth(&access.access_token)
        .send()
        .await?
        .error_for_status()?;
    //
    let output = output_base.join(file.name);
    let pdf = file_export(client, access, &file.id).await?;
    let mut file = io::BufWriter::new(fs::File::create(output).await?);
    file.write_all(&pdf).await?;
    file.flush().await?;
    file.into_inner().sync_all().await?;
    Ok(pdf)
}

/// Send the email
async fn send_email(client: &Client, access: &Access, pdf: &[u8], iso_time: &str) -> Result<()> {
    let url = Url::parse_with_params(GMAIL_SEND, &[("uploadType", "multipart")])?;

    let msg = format!(
        "\
From: {from}
To: {to}
Subject: {subject}
Content-Type: multipart/related; boundary=invoice_pdf

--invoice_pdf

Here is my invoice for the previous 2 weeks, thank you.

--invoice_pdf
Content-Type: application/pdf
Content-Transfer-Encoding: base64
Content-Disposition: attachment; filename=Invoice-{iso_time}.pdf

{}
--invoice_pdf--
    ",
        base64::encode(&pdf),
        to = INVOICE_EMAIL,
        from = get_email(client, access).await?,
        // TODO: Should be `Invoice - Name`
        subject = "Invoice",
        iso_time = iso_time,
    )
    .replace('\n', "\r\n");
    let len = msg.len();
    client
        .post(url)
        // .body(base64::encode_config(&pdf, URL_SAFE))
        // .body(base64::encode_config(&msg, URL_SAFE))
        .body(msg)
        // .json(&json!(
        //     // DianaNites@gmail.com
        //     {
        //         //
        //         // "raw": base64::encode_config(&pdf, URL_SAFE)
        //         "raw": base64::encode_config(msg, URL_SAFE)
        //     }
        // ))
        // .header(CONTENT_LENGTH, value)
        // .header(CONTENT_TYPE, "application/pdf")
        .header(CONTENT_TYPE, "message/rfc822")
        // .header(CONTENT_TYPE, "multipart/related; boundary=invoice_pdf")
        // .header(CONTENT_LENGTH, pdf.len())
        .header(CONTENT_LENGTH, len)
        .bearer_auth(&access.access_token)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let time = OffsetDateTime::now_utc();
    let sheets_time = time.format(format_description!("[month]/[day]/[year]"))?;
    let iso_time = time.format(format_description!("[year]-[month]-[day]"))?;
    let path = Path::new("./scratch/tokens.json");
    let output_base = Path::new("./scratch/invoices");
    fs::create_dir_all("./scratch").await?;
    let client = Client::builder().user_agent(APP_USER_AGENT).build()?;
    let mut access: Access = check_access(&client, path).await?;
    let (file, folder) = loop {
        match get_files(&client, &access).await {
            Ok(f) => break f,
            Err(_) => {
                access = refresh(&client, access, path).await?;
            }
        };
    };
    let pdf = ready_invoice(
        &client,
        &access,
        &file.id,
        &folder.id,
        &sheets_time,
        &iso_time,
        output_base,
    )
    .await?;

    send_email(&client, &access, &pdf, &iso_time).await?;

    Ok(())
}
