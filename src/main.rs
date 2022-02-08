#![allow(unused_imports, dead_code, unused_variables, unused_mut)]
use reqwest::header::CONTENT_LENGTH;
use reqwest::header::CONTENT_TYPE;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::{stdin, BufReader, BufWriter, Read};
use std::path::Path;
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

static FILE_LIST: &str = "https://www.googleapis.com/drive/v3/files";
static FILE_COPY: &str = "https://www.googleapis.com/drive/v3/files/fileId/copy";

static DRIVE_SCOPES: &[&str] = &["https://www.googleapis.com/auth/drive"];

#[derive(Debug, Serialize, Deserialize)]
struct Access {
    access_token: String,
    expires_in: u64,
    #[serde(default)]
    refresh_token: String,
    scope: String,
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListResponse {
    files: Vec<FileResource>,
}

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
    println!("{}", auth_url);
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
    // println!("{}", token_url);
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

async fn file_copy(file_id: &str) -> Result<Url> {
    Ok(Url::parse(&format!("{}/{}/copy", FILE_LIST, file_id))?)
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
        // template?.json::<ListResponse>(),
        template?.json::<Value>(),
        folder?.json::<ListResponse>(),
    );
    dbg!(template)?;
    todo!()
    // let (mut template, mut folder) = (template?, folder?);
    // dbg!(&template);
    // let (template, folder) = (template.files.swap_remove(0), folder.files.swap_remove(0));
    // Ok((template, folder))
}

#[tokio::main]
async fn main() -> Result<()> {
    let path = Path::new("./scratch/tokens.json");
    let client = Client::builder().user_agent(APP_USER_AGENT).build()?;
    let access: Access = if path.exists() {
        serde_json::from_reader(BufReader::new(fs::File::open(path)?))?
    } else {
        first_access(&client).await?
    };
    let (file, folder) = get_files(&client, &access).await?;
    //
    dbg!(&file);
    dbg!(&folder);
    let mut url = file_copy(&file.id).await?;
    println!("{}", url);
    // url.query_pairs_mut().extend_pairs(&[
    //     //
    //     // ("TEST", ""),
    //     // ("TEST", ""),
    //     ("", ""),
    // ]);
    // println!("{}", url);

    // application/vnd.google-apps.spreadsheet
    // application/vnd.google-apps.folder
    Ok(())
}
