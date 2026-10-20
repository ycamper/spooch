use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

use super::*;

#[test]
fn links_are_base62_and_unguessable() {
    let a = Link::new();
    assert_eq!((a.id.len(), a.secret.len()), (10, 22));
    assert!(
        (a.id.clone() + &a.secret)
            .bytes()
            .all(|b| ALPHABET.contains(&b))
    );
    assert_ne!(a, Link::new());
    let fixed = Link {
        id: "a".into(),
        secret: "b".into(),
    };
    assert_eq!(fixed.url("https://s.test/"), "https://s.test/ab");
    assert_eq!(
        sha256_hex("secret"),
        "2bb80d537b1da3e38bd30361aa855686bde0eacd7162fef6a25fe97bf527a25b"
    );
}

/// A one-off HTTP server that answers each request in turn with the
/// replies `replies` makes from its address, and reports each request's
/// first line and body.
fn serve(
    replies: impl FnOnce(&str) -> Vec<(u16, String)>,
) -> (String, mpsc::Receiver<(String, Vec<u8>)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    let replies = replies(&addr);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for (status, body) in replies {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut length = 0;
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                if h == "\r\n" {
                    break;
                }
                if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
            }
            let mut req_body = vec![0; length];
            reader.read_exact(&mut req_body).unwrap();
            let _ = tx.send((line.trim().to_string(), req_body));
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (addr, rx)
}

#[test]
fn a_share_creates_uploads_and_finalizes() {
    let (server, rx) = serve(|addr| {
        vec![
            (201, format!(r#"{{"upload_url":"{addr}/bucket/x?sig=1"}}"#)),
            (200, String::new()),
            (200, r#"{"state":"live"}"#.into()),
        ]
    });
    let c = Client::new(&server, "spd_abc");
    let (done_tx, done_rx) = mpsc::channel();
    let link = c.share(b"\x89PNG fake".to_vec(), Options::default(), move |r| {
        done_tx.send(r).unwrap()
    });
    assert!(
        link.starts_with(&server) && link.len() == server.len() + 33,
        "{link}"
    );
    let result = done_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(result, Ok(link.clone()));
    let (create, body) = rx.recv().unwrap();
    assert_eq!(create, "POST /v1/shares HTTP/1.1");
    let sent: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let id = &link[server.len() + 1..][..10];
    let secret = &link[server.len() + 11..];
    assert_eq!(sent["id"], id);
    assert_eq!(sent["secret_hash"], sha256_hex(secret));
    assert_eq!(sent["public"], true);
    let (upload, bytes) = rx.recv().unwrap();
    assert_eq!(upload, "PUT /bucket/x?sig=1 HTTP/1.1");
    assert_eq!(bytes, b"\x89PNG fake");
    let (finalize, _) = rx.recv().unwrap();
    assert_eq!(finalize, format!("POST /v1/shares/{id}/finalize HTTP/1.1"));
}

#[test]
fn errors_come_back_in_the_services_words() {
    let (server, _rx) = serve(|_| {
        vec![
            (401, r#"{"error":"that device token isn't valid"}"#.into()),
            (409, r#"{"error":"that id is taken; pick another"}"#.into()),
        ]
    });
    let c = Client::new(&server, "spd_x");
    assert!(c.me().unwrap_err().contains("sign in again"));
    let err = read::<serde_json::Value>(c.api("POST", "/shares").call()).unwrap_err();
    assert_eq!(String::from(err), "that id is taken; pick another");
}

#[test]
fn a_failed_upload_doesnt_tell_its_signed_url() {
    // Nothing listens on port 9 here, so the upload can't connect.
    let (server, _rx) = serve(|_| {
        vec![(
            201,
            r#"{"upload_url":"http://127.0.0.1:9/bucket/x?X-Amz-Signature=SECRETSIG"}"#.into(),
        )]
    });
    let (done_tx, done_rx) = mpsc::channel();
    Client::new(&server, "spd_abc").share(b"\x89PNG".to_vec(), Options::default(), move |r| {
        done_tx.send(r).unwrap()
    });
    let err = done_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap_err();
    assert!(err.starts_with("the upload failed"), "{err}");
    assert!(
        !err.contains("SECRETSIG") && !err.contains("127.0.0.1"),
        "{err}"
    );
}

#[test]
fn me_names_the_account_and_where_links_point() {
    let (server, rx) =
        serve(|_| vec![(200, r#"{"name":"mark","links":"https://img.test/"}"#.into())]);
    let c = Client::new(&server, "spd_abc");
    assert_eq!(c.me().unwrap(), "mark");
    assert_eq!(rx.recv().unwrap().0, "GET /v1/me HTTP/1.1");
    // Links now point where the service said.
    assert!(
        c.share(Vec::new(), Options::default(), |_| ())
            .starts_with("https://img.test/")
    );
}

#[test]
fn accounts_sign_in_and_change_passwords() {
    let (server, rx) = serve(|_| {
        vec![
            (
                201,
                r#"{"name":"dana","token":"spd_new","device":7}"#.into(),
            ),
            (401, r#"{"error":"wrong name or password"}"#.into()),
            (204, String::new()),
            (204, String::new()),
        ]
    });
    let made = create_account(&server, "dana", "a long password", "letmein", "laptop").unwrap();
    assert_eq!(
        made,
        SignedIn {
            token: "spd_new".into(),
            device: 7,
            name: "dana".into()
        }
    );
    let (line, body) = rx.recv().unwrap();
    assert_eq!(line, "POST /v1/accounts HTTP/1.1");
    let sent: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(sent["code"], "letmein");
    assert_eq!(sent["device"], "laptop");
    // A wrong password says so, not that a token is bad.
    assert_eq!(
        sign_in(&server, "dana", "wrong", "desk").unwrap_err(),
        "wrong name or password"
    );
    let c = Client::new(&server, "spd_new");
    c.set_password(None, "another long one").unwrap();
    rx.recv().unwrap();
    assert_eq!(rx.recv().unwrap().0, "PUT /v1/password HTTP/1.1");
    c.sign_out_device(7).unwrap();
    assert_eq!(rx.recv().unwrap().0, "DELETE /v1/devices/7 HTTP/1.1");
}

#[test]
fn a_capture_pushes_its_three_parts_then_finishes() {
    let (server, rx) = serve(|addr| {
        vec![
            (
                200,
                format!(
                    r#"{{"version":5,"uploads":{{"doc":"{addr}/u/doc","source":"{addr}/u/source","thumb":"{addr}/u/thumb"}},
                    "types":{{"doc":"application/json","source":"image/png","thumb":"image/png"}}}}"#
                ),
            ),
            (200, String::new()),
            (200, String::new()),
            (200, String::new()),
            (200, r#"{"version":5}"#.into()),
            (
                409,
                r#"{"error":"it changed on another device meanwhile","version":9}"#.into(),
            ),
        ]
    });
    let c = Client::new(&server, "spd_abc");
    let parts: (&[u8], &[u8], &[u8]) = (b"{}", b"\x89PNG source", b"\x89PNG thumb");
    let pushed = c
        .push_capture("abc", 0, "2026-10-18T12:00:00Z", parts)
        .unwrap();
    assert_eq!(pushed, Pushed::Done(5));
    let (begin, body) = rx.recv().unwrap();
    assert_eq!(begin, "PUT /v1/captures/abc HTTP/1.1");
    let sent: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        (sent["base"].as_i64(), sent["source_size"].as_u64()),
        (Some(0), Some(11))
    );
    let uploads: Vec<_> = (0..3).map(|_| rx.recv().unwrap()).collect();
    assert_eq!(
        uploads[1],
        ("PUT /u/source HTTP/1.1".into(), b"\x89PNG source".to_vec())
    );
    assert_eq!(
        rx.recv().unwrap().0,
        "POST /v1/captures/abc/finish HTTP/1.1"
    );
    // Another device got there first.
    assert_eq!(
        c.push_capture("abc", 5, "2026-10-18T12:00:00Z", parts)
            .unwrap(),
        Pushed::Conflict(9)
    );
}

#[test]
fn changes_are_read_a_page_at_a_time() {
    let (server, rx) = serve(|_| {
        vec![(
            200,
            r#"{"captures":[{"id":"a","version":3,"seq":7,"taken":"2026-10-18T12:00:00Z","updated":"x","size":5,"thumb_url":"https://t"},
            {"id":"b","version":2,"seq":8,"taken":"2026-10-17T09:30:15Z","updated":"x","size":0,"gone":true}],"next":8,"more":false}"#
                .into(),
        )]
    });
    let ch = Client::new(&server, "spd_abc").changes(0).unwrap();
    assert_eq!((ch.next, ch.more, ch.captures.len()), (8, false, 2));
    assert_eq!(ch.captures[0].thumb_url.as_deref(), Some("https://t"));
    assert!(ch.captures[1].gone && ch.captures[1].thumb_url.is_none());
    // A page the size of the service's default overruns the reply limit.
    assert_eq!(
        rx.recv().unwrap().0,
        "GET /v1/captures?since=0&limit=50 HTTP/1.1"
    );
    assert_eq!(new_capture_id().len(), 16);
}

#[test]
fn errors_are_told_apart_by_status() {
    let (server, _rx) = serve(|_| {
        vec![
            (404, r#"{"error":"whatever it says"}"#.into()),
            (410, r#"{"error":"gone, in other words"}"#.into()),
            (500, r#"{"error":"broken"}"#.into()),
            (401, r#"{"error":"wrong name or password"}"#.into()),
        ]
    });
    let c = Client::new(&server, "spd_x");
    // Already gone is as good as trashed.
    assert_eq!(c.trash_capture("a"), Ok(()));
    assert_eq!(c.trash_capture("a"), Ok(()));
    assert_eq!(c.trash_capture("a"), Err("broken".into()));
    // Signing in sends no token, so its 401 keeps the service's words.
    let err = sign_in(&server, "mark", "nope", "pc").err();
    assert_eq!(err.as_deref(), Some("wrong name or password"));
}
