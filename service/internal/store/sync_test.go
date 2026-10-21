package store

import (
	"context"
	"errors"
	"testing"
	"time"
)

func syncContract(t *testing.T, s Store) {
	ctx := context.Background()
	must := func(err error) {
		t.Helper()
		if err != nil {
			t.Fatal(err)
		}
	}
	is := func(err, want error) {
		t.Helper()
		if !errors.Is(err, want) {
			t.Fatalf("got %v, want %v", err, want)
		}
	}

	// Accounts: a password, or none for one made by hand.
	sam, err := s.AddAccount(ctx, "sam", "hash-a")
	must(err)
	_, err = s.AddAccount(ctx, "sam", "hash-b")
	is(err, ErrExists)
	if u, h, err := s.Password(ctx, "sam"); err != nil || u.ID != sam.ID || h != "hash-a" {
		t.Fatalf("password: %v %q %v", u, h, err)
	}
	old, err := s.AddUser(ctx, "old")
	must(err)
	if _, h, err := s.Password(ctx, "old"); err != nil || h != "" {
		t.Fatalf("a hand-made account has a password: %q %v", h, err)
	}
	must(s.SetPassword(ctx, old.ID, "hash-c"))
	if _, h, _ := s.Password(ctx, "old"); h != "hash-c" {
		t.Fatalf("set password: %q", h)
	}
	_, _, err = s.Password(ctx, "nobody")
	is(err, ErrNotFound)

	// Devices: only their own account can sign them out.
	d1, err := s.AddDevice(ctx, sam.ID, "laptop", "sync-h1")
	must(err)
	_, err = s.AddDevice(ctx, sam.ID, "desk", "sync-h2")
	must(err)
	if ds, _ := s.UserDevices(ctx, sam.ID); len(ds) != 2 {
		t.Fatalf("devices: %v", ds)
	}
	is(s.RevokeUserDevice(ctx, old.ID, d1.ID), ErrNotFound)
	must(s.RevokeUserDevice(ctx, sam.ID, d1.ID))
	is(s.RevokeUserDevice(ctx, sam.ID, d1.ID), ErrNotFound)
	if ds, _ := s.UserDevices(ctx, sam.ID); len(ds) != 1 || ds[0].Label != "desk" {
		t.Fatalf("devices after signing one out: %v", ds)
	}

	// A capture: begun, then finished, it shows in the changes.
	sizes := Sizes{Doc: 10, Source: 100, Thumb: 20}
	v1, replaced, err := s.BeginCapture(ctx, sam.ID, "c1", 0, t0, sizes, t0)
	must(err)
	if v1 == 0 || replaced != 0 {
		t.Fatalf("begin: %d %d", v1, replaced)
	}
	if ch, _ := s.Changes(ctx, sam.ID, 0, 10); len(ch) != 0 {
		t.Fatalf("an unfinished capture shows: %v", ch)
	}
	if n, _ := s.Usage(ctx, sam.ID); n != 130 {
		t.Fatalf("usage while uploading: %d", n)
	}
	before, err := s.FinishCapture(ctx, sam.ID, "c1", v1, t0.Add(time.Minute))
	must(err)
	if before.Version != 0 {
		t.Fatalf("before the first version: %d", before.Version)
	}
	_, err = s.FinishCapture(ctx, sam.ID, "c1", v1, t0)
	is(err, ErrNotFound)
	ch, err := s.Changes(ctx, sam.ID, 0, 10)
	must(err)
	if len(ch) != 1 || ch[0].Version != v1 || ch[0].Sizes != sizes || !ch[0].Taken.Equal(t0) {
		t.Fatalf("changes: %+v", ch)
	}
	seq := ch[0].Seq
	if other, _ := s.Changes(ctx, old.ID, 0, 10); len(other) != 0 {
		t.Fatalf("another account sees it: %v", other)
	}

	// A new version needs the live one as its base.
	_, _, err = s.BeginCapture(ctx, sam.ID, "c1", 0, t0, sizes, t0)
	is(err, ErrConflict)
	_, _, err = s.BeginCapture(ctx, sam.ID, "c2", 5, t0, sizes, t0)
	is(err, ErrConflict)
	v2, _, err := s.BeginCapture(ctx, sam.ID, "c1", v1, t0, Sizes{1, 2, 3}, t0)
	must(err)
	// Begun again (another device), the first upload is replaced.
	v3, replaced, err := s.BeginCapture(ctx, sam.ID, "c1", v1, t0, Sizes{4, 5, 6}, t0)
	must(err)
	if replaced != v2 || v3 <= v2 {
		t.Fatalf("replaced %d by %d, want %d", replaced, v3, v2)
	}
	_, err = s.FinishCapture(ctx, sam.ID, "c1", v2, t0)
	is(err, ErrNotFound)
	before, err = s.FinishCapture(ctx, sam.ID, "c1", v3, t0)
	must(err)
	if before.Version != v1 {
		t.Fatalf("replaced version %d, want %d", before.Version, v1)
	}
	ch, _ = s.Changes(ctx, sam.ID, seq, 10)
	if len(ch) != 1 || ch[0].Version != v3 || ch[0].Sizes != (Sizes{4, 5, 6}) {
		t.Fatalf("changes since %d: %+v", seq, ch)
	}

	// The trash: it can't change there, and comes back out.
	must(s.TrashCapture(ctx, sam.ID, "c1", t0))
	is(s.TrashCapture(ctx, sam.ID, "c1", t0), ErrNotFound)
	_, _, err = s.BeginCapture(ctx, sam.ID, "c1", v3, t0, sizes, t0)
	is(err, ErrTrashed)
	must(s.RestoreCapture(ctx, sam.ID, "c1"))
	is(s.RestoreCapture(ctx, sam.ID, "c1"), ErrNotFound)

	// Purged, only the record stays, and it says so in the changes.
	was, err := s.PurgeCapture(ctx, sam.ID, "c1")
	must(err)
	if was.Version != v3 || was.Gone {
		t.Fatalf("purged %+v", was)
	}
	_, _, err = s.BeginCapture(ctx, sam.ID, "c1", v3, t0, sizes, t0)
	is(err, ErrGone)
	ch, _ = s.Changes(ctx, sam.ID, 0, 10)
	if len(ch) != 1 || !ch[0].Gone {
		t.Fatalf("a purged capture's change: %+v", ch)
	}
	if n, _ := s.Usage(ctx, sam.ID); n != 0 {
		t.Fatalf("usage after purging: %d", n)
	}

	// The sweeper purges what's a month in the trash and drops uploads
	// left unfinished; fresher ones stay.
	for _, id := range []string{"old-trash", "new-trash", "half-done", "abandoned"} {
		v, _, err := s.BeginCapture(ctx, sam.ID, id, 0, t0, sizes, t0)
		must(err)
		if id != "abandoned" {
			_, err = s.FinishCapture(ctx, sam.ID, id, v, t0)
			must(err)
		}
	}
	must(s.TrashCapture(ctx, sam.ID, "old-trash", t0))
	must(s.TrashCapture(ctx, sam.ID, "new-trash", t0.AddDate(0, 0, 29)))
	live, _ := s.Capture(ctx, sam.ID, "half-done")
	_, _, err = s.BeginCapture(ctx, sam.ID, "half-done", live.Version, t0, sizes, t0)
	must(err)
	purged, dropped, err := s.SweepCaptures(ctx, t0.AddDate(0, 0, 1), t0.Add(time.Hour), 10)
	must(err)
	if len(purged) != 1 || purged[0].ID != "old-trash" {
		t.Fatalf("purged %+v", purged)
	}
	if len(dropped) != 2 {
		t.Fatalf("dropped %+v", dropped)
	}
	if c, _ := s.Capture(ctx, sam.ID, "half-done"); c.Pending != 0 || c.Version != live.Version {
		t.Fatalf("a dropped upload took the capture with it: %+v", c)
	}
	_, err = s.Capture(ctx, sam.ID, "abandoned")
	is(err, ErrNotFound)
	if c, _ := s.Capture(ctx, sam.ID, "new-trash"); c.Gone {
		t.Fatal("a fresh trashing was purged")
	}

	// Live and pending shares count toward the quota; deleted ones don't,
	// but still count toward the day's shares.
	base, err := s.Usage(ctx, sam.ID)
	must(err)
	share := Share{ID: "samshare01", Owner: sam.ID, SecretHash: "h", Size: 7, Type: "image/png",
		Created: t0, State: Pending, Public: true}
	must(s.CreateShare(ctx, share, nil))
	for _, state := range []State{Pending, Live} {
		if state == Live {
			must(s.Move(ctx, share.ID, Live, Pending))
		}
		if n, _ := s.Usage(ctx, sam.ID); n != base+7 {
			t.Fatalf("usage with a %s share: %d, want %d", state, n, base+7)
		}
	}
	must(s.Move(ctx, share.ID, Deleted, Live))
	if n, _ := s.Usage(ctx, sam.ID); n != base {
		t.Fatalf("usage after deleting the share: %d, want %d", n, base)
	}
	if n, _ := s.SharesSince(ctx, sam.ID, t0); n != 1 {
		t.Fatalf("shares since it was made: %d", n)
	}
	if n, _ := s.SharesSince(ctx, sam.ID, t0.Add(time.Second)); n != 0 {
		t.Fatalf("shares since after it was made: %d", n)
	}
}
