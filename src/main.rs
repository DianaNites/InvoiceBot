use reqwest::{
    header::{CONTENT_LENGTH, CONTENT_TYPE},
    Client,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::{stdin, BufReader, BufWriter, Write},
    path::Path,
};
use time::{macros::format_description, OffsetDateTime};
use tokio::join;
use url::Url;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

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

/// Scopes our tokens need
static DRIVE_SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/drive",
    // "https://www.googleapis.com/auth/spreadsheets",
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

/// Save
fn save_access(access: Access) -> Result<Access> {
    serde_json::to_writer_pretty(
        BufWriter::new(fs::File::create("./scratch/tokens.json")?),
        &access,
    )?;
    Ok(access)
}

async fn first_access(client: &Client) -> Result<Access> {
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
    let mut auth = String::new();
    stdin().read_line(&mut auth)?;
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
    println!("{:#?}", text);
    save_access(text)
}

async fn refresh(client: &Client, access: Access) -> Result<Access> {
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
    save_access(Access {
        refresh_token: access.refresh_token,
        ..text
    })
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
        //
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
        &[
            //
            ("fields", "id, name, mimeType, parents, webViewLink"),
        ],
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
    dbg!(&json);
    Ok(json)
}

/// Export invoice to PDF
async fn file_export(client: &Client, access: &Access, file_id: &str) -> Result<Vec<u8>> {
    let url = Url::parse_with_params(
        &format!("{}/{}/export", FILE_LIST, file_id),
        &[
            //
            ("mimeType", "application/pdf"),
            // ("fields", "id, name, mimeType, parents, webViewLink"),
        ],
    )?;
    let res = client
        .get(url)
        .bearer_auth(&access.access_token)
        .send()
        .await?;
    let json = res.bytes().await?;
    // dbg!(&json);
    Ok(json.to_vec())
}

#[tokio::main]
async fn main() -> Result<()> {
    let time = OffsetDateTime::now_utc();
    let sheets_time = time.format(format_description!("[month]/[day]/[year]"))?;
    let iso_time = time.format(format_description!("[year]-[month]-[day]"))?;
    let path = Path::new("./scratch/tokens.json");
    let client = Client::builder().user_agent(APP_USER_AGENT).build()?;
    let mut access: Access = if path.exists() {
        serde_json::from_reader(BufReader::new(fs::File::open(path)?))?
    } else {
        first_access(&client).await?
    };
    let (file, folder) = loop {
        match get_files(&client, &access).await {
            Ok(f) => break f,
            Err(_) => {
                access = refresh(&client, access).await?;
            }
        };
    };
    //
    dbg!(&file);
    dbg!(&folder);
    let file = file_copy(&client, &access, &folder.id, &file.id, &iso_time).await?;
    dbg!(&file);
    let url = Url::parse_with_params(
        &format!("{}/{}/values/D9:E9", SPREADSHEET_BASE, file.id),
        &[
            //
            ("valueInputOption", "USER_ENTERED"),
            // ("includeGridData", "true"),
            // ("ranges", "D9:E9"),
        ],
    )?;
    let res = client
        .put(url)
        .json(&json!({ "values": [[sheets_time]] }))
        .bearer_auth(&access.access_token)
        .send()
        .await?;
    let json = res.json::<Value>().await?;
    dbg!(json);

    let pdf = file_export(&client, &access, &file.id).await?;
    let file = fs::File::create("./scratch/test.pdf")?;
    let mut file = BufWriter::new(file);
    file.write_all(&pdf)?;
    file.flush()?;
    file.into_inner()?.sync_all()?;

    Ok(())
}
