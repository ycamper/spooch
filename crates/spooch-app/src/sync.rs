//! Syncing captures with the signed-in account, on one background thread:
//! every minute, and whenever a capture is kept, it pushes what changed
//! here and pulls what changed elsewhere. Captures from other devices
//! arrive as stubs (a thumbnail and a note); opening one downloads the
//! rest. Nothing goes anywhere until syncing is switched on.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use spooch_platform::log;
use spooch_platform::recents::{Recents, SyncNote};
use spooch_platform::save::{self, LocalTime};
use spooch_share::{Client, Pushed};

use crate::Sharing;

/// How often a round runs when nothing asks for one sooner.
const ROUND: Duration = Duration::from_secs(60);

/// Rounds a change that can't be applied holds the pull back before it's
/// skipped, so one bad capture can't stop sync for good.
const TRIES: u32 = 10;

/// Failed tries per change (capture id and version).
type Stuck = HashMap<(String, i64), u32>;

/// A job for the sync thread.
enum Job {
    /// Run a round now.
    Now,
    /// Put this capture (by its id in the account) in the account's trash.
    Trash(String),
    /// Download this stub (by its id in Recents) so it can open.
    Fetch(String, Box<dyn FnOnce(Result<(), String>) + Send>),
}

/// The shell keeps one, in `Carry`; clones share the thread. The default
/// one has no thread and does nothing (no Recents).
#[derive(Clone, Default)]
pub struct Syncing {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    on: AtomicBool,
    jobs: Mutex<Option<Sender<Job>>>,
    /// Bumped whenever Recents changed under the gallery.
    changes: AtomicU64,
    /// Windows to wake when it does.
    wakers: Mutex<Vec<Weak<dyn Fn() + Send + std::marker::Sync>>>,
}

impl Syncing {
    /// Sync `recents` with the account `sharing` is signed in to, once
    /// `on`.
    pub fn start(recents: Option<Recents>, sharing: Sharing, on: bool) -> Syncing {
        let sync = Syncing::default();
        sync.inner.on.store(on, Ordering::Relaxed);
        let Some(recents) = recents else {
            return sync;
        };
        let (tx, rx) = mpsc::channel::<Job>();
        *sync.inner.jobs.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
        let inner = Arc::downgrade(&sync.inner);
        std::thread::spawn(move || {
            let mut stuck = Stuck::new();
            loop {
                let job = rx.recv_timeout(ROUND);
                let Some(inner) = inner.upgrade() else { break };
                let client = sharing.client();
                match job {
                    Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) | Ok(Job::Now) => {}
                    Ok(Job::Trash(id)) => {
                        if let Some(c) = &client {
                            match c.trash_capture(&id) {
                                Ok(()) => log::line("sync: put a capture in the account's trash"),
                                Err(e) => log::line(format!("sync: trashing: {e}")),
                            }
                        }
                        continue;
                    }
                    Ok(Job::Fetch(local, done)) => {
                        let got = match &client {
                            Some(c) => fetch(&recents, c, &local),
                            None => Err("sign in to download it".into()),
                        };
                        if let Err(e) = &got {
                            log::line(format!("sync: downloading {local}: {e}"));
                        }
                        done(got);
                        continue;
                    }
                }
                if let (true, Some(c)) = (inner.on.load(Ordering::Relaxed), client)
                    && round(&recents, &c, &mut stuck)
                {
                    inner.changed();
                }
            }
        });
        sync
    }

    pub fn is_on(&self) -> bool {
        self.inner.on.load(Ordering::Relaxed)
    }

    /// Switch syncing on (a round runs at once) or off.
    pub fn set_on(&self, on: bool) {
        self.inner.on.store(on, Ordering::Relaxed);
        if on {
            self.kick();
        }
    }

    fn send(&self, job: Job) -> bool {
        let jobs = self.inner.jobs.lock().unwrap_or_else(|e| e.into_inner());
        jobs.as_ref().is_some_and(|tx| tx.send(job).is_ok())
    }

    /// Run a round soon: a capture was kept.
    pub fn kick(&self) {
        if self.is_on() {
            self.send(Job::Now);
        }
    }

    /// Kept capture `local` was deleted: put it in the account's trash too,
    /// if it's synced. Call it before deleting, while its note is there.
    pub fn trash(&self, recents: &Recents, local: &str) {
        if let (true, Some(note)) = (self.is_on(), recents.sync_note(local)) {
            self.send(Job::Trash(note.id));
        }
    }

    /// Download stub `local` so it can open; `done` hears how it went, on
    /// the sync thread.
    pub fn fetch(&self, local: &str, done: impl FnOnce(Result<(), String>) + Send + 'static) {
        let job = Job::Fetch(local.to_string(), Box::new(done));
        if !self.send(job) {
            log::line("sync: there's no sync to download with");
        }
    }

    /// A number that moves on whenever sync changed Recents, so a gallery
    /// knows to look again.
    pub fn changes(&self) -> u64 {
        self.inner.changes.load(Ordering::Relaxed)
    }

    /// Wake with `wake` whenever sync changes Recents, while it lives.
    pub fn watch(&self, wake: &Arc<dyn Fn() + Send + std::marker::Sync>) {
        let mut wakers = self.inner.wakers.lock().unwrap_or_else(|e| e.into_inner());
        wakers.retain(|w| w.strong_count() > 0);
        wakers.push(Arc::downgrade(wake));
    }
}

impl Inner {
    fn changed(&self) {
        self.changes.fetch_add(1, Ordering::Relaxed);
        let wakers = self.wakers.lock().unwrap_or_else(|e| e.into_inner());
        for wake in wakers.iter().filter_map(Weak::upgrade) {
            wake();
        }
    }
}

/// One round: push what changed here, then pull what changed elsewhere.
/// Returns whether Recents (or what's synced) changed.
fn round(recents: &Recents, client: &Client, stuck: &mut Stuck) -> bool {
    let pushed = push(recents, client);
    match pull(recents, client, stuck) {
        Ok(changed) => pushed || changed,
        Err(e) => {
            log::line(format!("sync: {e}"));
            pushed
        }
    }
}

/// Push every kept capture new or changed since its last push. Returns
/// whether any went up.
fn push(recents: &Recents, client: &Client) -> bool {
    let mut any = false;
    for local in recents.list() {
        let Some(stamp) = recents.stamp(&local) else {
            continue;
        };
        let note = recents.sync_note(&local);
        if note.as_ref().is_some_and(|n| n.stamp == stamp) {
            continue;
        }
        // A capture kept before its thumbnail is made waits a round.
        let Ok(files) = recents.files(&local) else {
            continue;
        };
        let id = note
            .as_ref()
            .map_or_else(spooch_share::new_capture_id, |n| n.id.clone());
        let taken = taken_at(&local)
            .unwrap_or_else(|| save::utc_from_unix((stamp / 1_000_000_000) as u64).rfc3339());
        let parts = (&files.0[..], &files.1[..], &files.2[..]);
        let mut base = note.as_ref().map_or(0, |n| n.version);
        // Another device's version meanwhile: this edit is newer, so it
        // goes on top.
        let mut pushed = client.push_capture(&id, base, &taken, parts);
        if let Ok(Pushed::Conflict(now)) = pushed {
            base = now;
            pushed = client.push_capture(&id, base, &taken, parts);
        }
        match pushed {
            Ok(Pushed::Done(version)) => {
                let note = SyncNote { id, version, stamp };
                if let Err(e) = recents.write_sync_note(&local, &note) {
                    log::line(format!("sync: noting {local}: {e}"));
                }
                log::line(format!("sync: pushed {local}"));
                any = true;
            }
            Ok(Pushed::Conflict(_)) => log::line(format!("sync: {local} keeps changing")),
            Err(e) => {
                log::line(format!("sync: pushing {local}: {e}"));
                // The account's full, or the service is out: try later.
                return any;
            }
        }
    }
    any
}

/// Take what changed in the account since the last pull: new captures as
/// stubs, newer versions of ones here, and ones gone. Returns whether
/// Recents changed.
fn pull(recents: &Recents, client: &Client, stuck: &mut Stuck) -> Result<bool, String> {
    let mut here: HashMap<String, String> = recents
        .list_all()
        .into_iter()
        .filter_map(|local| Some((recents.sync_note(&local)?.id, local)))
        .collect();
    let mut since = recents.sync_cursor();
    let mut changed = false;
    loop {
        let page = client.changes(since)?;
        // The cursor stops before the first change that couldn't be
        // applied, so the next round tries it again (up to `TRIES`).
        let mut failed = false;
        for c in &page.captures {
            match here.get(&c.id).cloned() {
                // Trashed or gone in the account: gone here too.
                Some(local) if c.gone || c.trashed.is_some() => match recents.delete(&local) {
                    Ok(()) => {
                        here.remove(&c.id);
                        changed = true;
                        log::line(format!("sync: {local} was deleted on another device"));
                    }
                    Err(e) => {
                        log::line(format!("sync: deleting {local}: {e}"));
                        failed = !give_up(stuck, c);
                    }
                },
                None if c.gone || c.trashed.is_some() => {}
                Some(local) => {
                    let note = recents.sync_note(&local);
                    if note.is_some_and(|n| n.version >= c.version) {
                        continue;
                    }
                    // An edit here not pushed yet goes on top at the next
                    // push, so don't overwrite it.
                    if !recents.is_stub(&local) && !recents.up_to_date(&local) {
                        continue;
                    }
                    let note = SyncNote {
                        id: c.id.clone(),
                        version: c.version,
                        stamp: 0,
                    };
                    let got = if recents.is_stub(&local) {
                        thumb(client, c).and_then(|t| {
                            recents
                                .write_stub(&local, &t, &note)
                                .map_err(|e| e.to_string())
                        })
                    } else {
                        download(client, &c.id).and_then(|parts| {
                            let parts = (&parts.0[..], &parts.1[..], &parts.2[..]);
                            recents
                                .write_synced(&local, parts, &note)
                                .map_err(|e| e.to_string())
                        })
                    };
                    match got {
                        Ok(()) => {
                            changed = true;
                            log::line(format!("sync: {local} changed on another device"));
                        }
                        Err(e) => {
                            log::line(format!("sync: updating {local}: {e}"));
                            failed = !give_up(stuck, c);
                        }
                    }
                }
                None => {
                    let local = recents.new_id(&taken(&c.taken));
                    let note = SyncNote {
                        id: c.id.clone(),
                        version: c.version,
                        stamp: 0,
                    };
                    match thumb(client, c).and_then(|t| {
                        recents
                            .write_stub(&local, &t, &note)
                            .map_err(|e| e.to_string())
                    }) {
                        Ok(()) => {
                            here.insert(c.id.clone(), local.clone());
                            changed = true;
                            log::line(format!("sync: {local} came from another device"));
                        }
                        Err(e) => {
                            log::line(format!("sync: a new capture: {e}"));
                            failed = !give_up(stuck, c);
                        }
                    }
                }
            }
            if failed {
                break;
            }
            since = c.seq;
        }
        if !failed {
            since = page.next;
            stuck.clear();
        }
        recents
            .set_sync_cursor(since)
            .map_err(|e| format!("keeping the sync cursor: {e}"))?;
        if failed || !page.more {
            return Ok(changed);
        }
    }
}

/// Count another failed try at change `c`: whether it has had enough and
/// the pull should go past it.
fn give_up(stuck: &mut Stuck, c: &spooch_share::Change) -> bool {
    let tries = stuck.entry((c.id.clone(), c.version)).or_default();
    *tries += 1;
    if *tries < TRIES {
        return false;
    }
    log::line(format!(
        "sync: skipping a change to {} after {TRIES} tries",
        c.id
    ));
    true
}

/// A change's thumbnail.
fn thumb(client: &Client, c: &spooch_share::Change) -> Result<Vec<u8>, String> {
    let url = c.thumb_url.as_deref().ok_or("no thumbnail")?;
    client.download(url)
}

/// A capture's three parts: document, image and thumbnail.
type Parts = (Vec<u8>, Vec<u8>, Vec<u8>);

/// A capture's parts from the account.
fn download(client: &Client, id: &str) -> Result<Parts, String> {
    Ok((
        client.capture_part(id, "doc")?,
        client.capture_part(id, "source")?,
        client.capture_part(id, "thumb")?,
    ))
}

/// Download stub `local`'s document and image, so it opens.
fn fetch(recents: &Recents, client: &Client, local: &str) -> Result<(), String> {
    let note = recents.sync_note(local).ok_or("it isn't synced")?;
    let parts = download(client, &note.id)?;
    recents
        .write_synced(local, (&parts.0, &parts.1, &parts.2), &note)
        .map_err(|e| e.to_string())?;
    log::line(format!("sync: downloaded {local}"));
    Ok(())
}

/// When kept capture `local` was taken, from its id ("2026-10-18 12.00.01"),
/// as the account keeps it: the time on the clock where it was taken, so
/// every device shows it as that.
fn taken_at(local: &str) -> Option<String> {
    let id = local.get(..19)?;
    let (date, time) = id.split_once(' ')?;
    let time = time.replace('.', ":");
    (date.len() == 10 && time.len() == 8).then(|| format!("{date}T{time}Z"))
}

/// When a capture was taken, as a Recents id's time.
fn taken(rfc3339: &str) -> LocalTime {
    LocalTime::parse_rfc3339(rfc3339).unwrap_or_else(|| save::utc_from_unix(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stuck_change_is_skipped_after_enough_tries() {
        let change = |version| spooch_share::Change {
            id: "c".into(),
            version,
            seq: 1,
            taken: String::new(),
            trashed: None,
            gone: false,
            thumb_url: None,
        };
        let mut stuck = Stuck::new();
        for _ in 1..TRIES {
            assert!(!give_up(&mut stuck, &change(1)));
        }
        // A newer version counts afresh.
        assert!(!give_up(&mut stuck, &change(2)));
        assert!(give_up(&mut stuck, &change(1)));
    }
    use spooch_platform::recents::Pixels;

    /// Two devices syncing through a real service: set
    /// SPOOCH_SYNC_TEST_SERVER (and SPOOCH_SYNC_TEST_CODE, its sign-up
    /// code) to run it against a throwaway one.
    #[test]
    #[ignore]
    fn two_devices_sync_through_the_service() {
        let server = std::env::var("SPOOCH_SYNC_TEST_SERVER").expect("a server to test with");
        let code = std::env::var("SPOOCH_SYNC_TEST_CODE").expect("its sign-up code");
        let name = format!("synctest{}", spooch_share::new_capture_id().to_lowercase());
        let signed =
            spooch_share::create_account(&server, &name[..20], "a test password", &code, "a")
                .expect("an account");
        let a_client = Client::new(&server, &signed.token);
        let b_client = Client::new(&server, &signed.token);
        let dir = |n: &str| {
            let d = std::env::temp_dir().join(format!("spooch-sync-{n}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            Recents::new(d)
        };
        let (a, b) = (dir("a"), dir("b"));
        let px = |v: u8| Pixels {
            width: 4,
            height: 3,
            bgra: vec![v; 48],
        };
        let t = LocalTime {
            year: 2026,
            month: 10,
            day: 18,
            hour: 12,
            minute: 0,
            second: 1,
        };

        // A keeps a capture; it goes up, and comes to B as a stub.
        let local = a.new_id(&t);
        a.write(&local, r#"{"v":1}"#, Some(&px(1))).unwrap();
        a.write_thumb(&local, &px(2)).unwrap();
        push(&a, &a_client);
        let note = a.sync_note(&local).expect("pushed and noted");
        assert!(
            pull(&b, &b_client, &mut Stuck::new()).unwrap(),
            "B got something"
        );
        let stubs: Vec<_> = b.stubs().into_iter().collect();
        // Named for when it was taken, as it is on A.
        assert_eq!(stubs, std::slice::from_ref(&local));
        let on_b = stubs[0].clone();
        // Pulling again finds nothing new; A's own pull neither.
        assert!(!pull(&b, &b_client, &mut Stuck::new()).unwrap());
        assert!(!pull(&a, &a_client, &mut Stuck::new()).unwrap());

        // Opened on B, it downloads, and is the same as on A.
        fetch(&b, &b_client, &on_b).unwrap();
        assert!(!b.is_stub(&on_b));
        assert_eq!(b.files(&on_b).unwrap(), a.files(&local).unwrap());

        // Edited on A, B gets the new version.
        std::thread::sleep(Duration::from_millis(20));
        a.write(&local, r#"{"v":2}"#, None).unwrap();
        push(&a, &a_client);
        assert!(a.sync_note(&local).unwrap().version > note.version);
        assert!(pull(&b, &b_client, &mut Stuck::new()).unwrap());
        assert_eq!(b.files(&on_b).unwrap().0, br#"{"v":2}"#);

        // Deleted on B, it goes to the trash, and from A.
        let b_sync = b.sync_note(&on_b).unwrap();
        b_client.trash_capture(&b_sync.id).unwrap();
        b.delete(&on_b).unwrap();
        assert!(pull(&a, &a_client, &mut Stuck::new()).unwrap());
        assert_eq!(a.list_all(), Vec::<String>::new());

        // Gone for good, so the test leaves nothing behind.
        a_client.delete_capture_forever(&b_sync.id).unwrap();
        let _ = a_client.sign_out_device(signed.device);
        let _ = std::fs::remove_dir_all(a.root());
        let _ = std::fs::remove_dir_all(b.root());
    }
}
