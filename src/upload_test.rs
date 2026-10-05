use super::{multipart, parse_link, BOUNDARY};

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
