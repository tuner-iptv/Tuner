//! Smoke test against a real mpv binary. Ignored by default so CI and
//! `cargo test` never need mpv; run with `cargo test -p tuner-mpv -- --ignored`.

use serde_json::json;
use tuner_mpv::{Mpv, MpvOptions};

#[test]
#[ignore = "needs an mpv binary"]
fn drives_a_real_mpv() {
    let opts = MpvOptions {
        extra_args: vec![
            "--vo=null".into(),
            "--ao=null".into(),
            "--force-window=no".into(),
        ],
        ..MpvOptions::default()
    };
    let mut mpv = Mpv::spawn(&opts).expect("spawn mpv");
    assert!(mpv.is_running());
    mpv.load("av://lavfi:testsrc").expect("loadfile");
    mpv.toggle_pause().expect("pause");
    assert!(mpv.is_paused().expect("pause state"));
    mpv.stop().expect("stop");
    drop(mpv);
}

#[test]
#[ignore = "needs an mpv binary"]
fn reports_an_unreachable_stream() {
    use std::time::{Duration, Instant};
    use tuner_mpv::StreamEnd;

    let opts = MpvOptions {
        extra_args: vec![
            "--vo=null".into(),
            "--ao=null".into(),
            "--force-window=no".into(),
        ],
        ..MpvOptions::default()
    };
    let mut mpv = Mpv::spawn(&opts).expect("spawn mpv");
    // Nothing listens on the discard port.
    mpv.load("http://127.0.0.1:9/live.ts").expect("loadfile");
    let deadline = Instant::now() + Duration::from_secs(15);
    let end = loop {
        if let Some(end) = mpv.poll() {
            break end;
        }
        assert!(Instant::now() < deadline, "no failure reported");
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(matches!(end, StreamEnd::Failed(_)), "{end:?}");

    // A playable source is not reported.
    mpv.load("av://lavfi:testsrc").expect("loadfile");
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(mpv.poll(), None);
}

#[test]
#[ignore = "needs an mpv binary"]
fn reports_a_server_that_answers_but_sends_nothing() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};
    use tuner_mpv::{CONNECT_TIMEOUT, LoadStage};

    // Accepts, sends headers, then goes silent: a hung IPTV panel.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut conn in listener.incoming().flatten() {
            let _ = conn.read(&mut [0; 4096]);
            let _ = conn.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\n\r\n");
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(60));
                drop(conn);
            });
        }
    });

    let opts = MpvOptions {
        extra_args: vec![
            "--vo=null".into(),
            "--ao=null".into(),
            "--force-window=no".into(),
        ],
        ..MpvOptions::default()
    };
    let mut mpv = Mpv::spawn(&opts).expect("spawn mpv");
    let started = Instant::now();
    mpv.load(&format!("http://127.0.0.1:{port}/live.ts"))
        .expect("loadfile");
    assert_eq!(mpv.stage(), Some(LoadStage::Connecting));
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(mpv.poll(), None);
    // No window while it connects.
    assert_eq!(
        mpv.command(&[json!("get_property"), json!("vo-configured")])
            .unwrap(),
        json!(false)
    );
    let end = loop {
        if let Some(end) = mpv.poll() {
            break end;
        }
        assert!(
            started.elapsed() < CONNECT_TIMEOUT + Duration::from_secs(2),
            "no failure reported"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    eprintln!("reported after {:?}: {end}", started.elapsed());
}

#[test]
#[ignore = "needs an mpv binary"]
fn audio_only_stream_gets_a_window_once_it_plays() {
    use std::time::{Duration, Instant};
    use tuner_mpv::LoadStage;

    // tuner's own --force-window=no applies; only output is nulled.
    let opts = MpvOptions {
        extra_args: vec!["--vo=null".into(), "--ao=null".into()],
        ..MpvOptions::default()
    };
    let mut mpv = Mpv::spawn(&opts).expect("spawn mpv");
    let vo = |mpv: &mut Mpv| {
        mpv.command(&[json!("get_property"), json!("vo-configured")])
            .unwrap()
    };
    mpv.load("av://lavfi:sine").expect("loadfile");
    let deadline = Instant::now() + Duration::from_secs(5);
    while mpv.stage() != Some(LoadStage::Playing) {
        assert_eq!(mpv.poll(), None);
        assert!(Instant::now() < deadline, "never played");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(mpv.poll(), None);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(vo(&mut mpv), json!(true));

    // The next load drops the forced window again.
    mpv.load("av://lavfi:sine").expect("loadfile");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(mpv.stage(), Some(LoadStage::Connecting));
    let forced = mpv
        .command(&[json!("get_property"), json!("force-window")])
        .unwrap();
    assert_eq!(forced, json!(false));
}

#[test]
#[ignore = "needs an mpv binary and no route to 10.255.255.1"]
fn redirect_to_a_dead_stream_server_fails_fast_with_a_reason() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};
    use tuner_mpv::StreamEnd;

    // An Xtream panel answering with a redirect to an unreachable
    // load-balancer.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = requests.clone();
    std::thread::spawn(move || {
        for mut conn in listener.incoming().flatten() {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let _ = conn.read(&mut [0; 4096]);
            let _ = conn.write_all(
                b"HTTP/1.1 302 Found\r\nLocation: http://10.255.255.1:8080/live/u/secret/1.ts\r\n\
                  Content-Length: 0\r\n\r\n",
            );
        }
    });

    let opts = MpvOptions {
        extra_args: vec!["--vo=null".into(), "--ao=null".into()],
        ..MpvOptions::default()
    };
    let mut mpv = Mpv::spawn(&opts).expect("spawn mpv");
    let started = Instant::now();
    mpv.load(&format!("http://127.0.0.1:{port}/live/u/secret/1.ts"))
        .expect("loadfile");
    let end = loop {
        if let Some(end) = mpv.poll() {
            break end;
        }
        // yt-dlp's fallback used to add 20 s on top of mpv's 10 s.
        assert!(started.elapsed() < Duration::from_secs(15), "too slow");
        std::thread::sleep(Duration::from_millis(100));
    };
    eprintln!("reported after {:?}: {end}", started.elapsed());
    let StreamEnd::Failed(reason) = end else {
        panic!("{end:?}");
    };
    assert!(reason.contains("10.255.255.1:8080"), "{reason}");
    assert!(!reason.contains("secret"), "{reason}");
    // One request to the provider: nothing retried it.
    assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), 1);
}
