use std::{borrow::Cow, env, time::Duration};

use anyhow::{bail, Context, Result};
use serde_json::Value;

const ENDPOINT: &str = "https://api.imgur.com/3/image";
const CLIENT_ID_VAR: &str = "IMGUR_CLIENT_ID";
const BOUNDARY: &str = "slightshot-multipart-7f3a";

const MULTIPART_HEADER: &[u8] = b"--slightshot-multipart-7f3a\r\n\
Content-Disposition: form-data; name=\"type\"\r\n\
\r\n\
file\r\n\
--slightshot-multipart-7f3a\r\n\
Content-Disposition: form-data; name=\"image\"; filename=\"capture.png\"\r\n\
Content-Type: image/png\r\n\
\r\n";

const MULTIPART_FOOTER: &[u8] = b"\r\n--slightshot-multipart-7f3a--\r\n";

pub fn upload(png: &[u8]) -> Result<String> {
  let client_id = env::var(CLIENT_ID_VAR).with_context(|| {
    format!(
      "{CLIENT_ID_VAR} is not set; register a Client-ID at \
       https://api.imgur.com/oauth2/addclient"
    )
  })?;
  let body = request(&client_id, png)?;
  parse_link(&body)
}

fn request(client_id: &str, png: &[u8]) -> Result<String> {
  let mut response = ureq::post(ENDPOINT)
    .config()
    .timeout_global(Some(Duration::from_secs(30)))
    .http_status_as_error(false)
    .build()
    .header("Authorization", &format!("Client-ID {client_id}"))
    .header(
      "Content-Type",
      &format!("multipart/form-data; boundary={BOUNDARY}"),
    )
    .send(&multipart(png))
    .context("the Imgur upload request failed")?;
  response
    .body_mut()
    .read_to_string()
    .context("cannot read the Imgur reply body")
}

fn multipart(png: &[u8]) -> Vec<u8> {
  let mut body = Vec::with_capacity(
    MULTIPART_HEADER.len() + png.len() + MULTIPART_FOOTER.len(),
  );
  body.extend_from_slice(MULTIPART_HEADER);
  body.extend_from_slice(png);
  body.extend_from_slice(MULTIPART_FOOTER);
  body
}

fn parse_link(body: &str) -> Result<String> {
  let json: Value = serde_json::from_str(body)
    .with_context(|| format!("Imgur sent non-JSON: {}", preview(body)))?;
  if !json["success"].as_bool().unwrap_or(false) {
    bail!(
      "Imgur rejected the upload: {}",
      describe(&json["data"]["error"])
    );
  }
  json["data"]["link"]
    .as_str()
    .map(str::to_owned)
    .context("Imgur replied without an image link")
}

fn describe<'a>(error: &'a Value) -> Cow<'a, str> {
  match error {
    Value::Null => Cow::Borrowed("unknown error"),
    Value::String(s) => Cow::Borrowed(s.as_str()),
    other => Cow::Owned(other.to_string()),
  }
}

fn preview(body: &str) -> &str {
  match body.char_indices().nth(300) {
    Some((idx, _)) => &body[..idx],
    None => body,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn success_reply_yields_the_image_link() {
    let link = parse_link(
      r#"{"data":{"id":"abc","link":"https://i.imgur.com/abc.png",
        "deletehash":"del"},"success":true,"status":200}"#,
    )
    .unwrap();
    assert_eq!(link, "https://i.imgur.com/abc.png");
  }

  #[test]
  fn rejected_reply_names_the_reason() {
    let error = parse_link(
      r#"{"data":{"error":"Invalid client_id"},"success":false,"status":403}"#,
    )
    .unwrap_err();
    assert!(error.to_string().contains("Invalid client_id"));
  }

  #[test]
  fn multipart_carries_png_and_boundary() {
    let png = vec![1, 2, 3];
    let body = multipart(&png);
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains(BOUNDARY));
    assert!(text.contains("filename=\"capture.png\""));
    assert!(text.ends_with(&format!("\r\n--{BOUNDARY}--\r\n")));
    assert!(body.windows(png.len()).any(|w| w == png.as_slice()));
  }
}
