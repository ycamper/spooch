package store

import (
	"context"
	"errors"
	"os"
	"slices"
	"testing"
	"time"
)

// Both stores pass the same tests. Postgres runs when
// SPOOCH_TEST_DATABASE_URL names an empty database (CI sets one).
func stores(t *testing.T) map[string]Store {
	out := map[string]Store{"memory": NewMemory()}
	if url := os.Getenv("SPOOCH_TEST_DATABASE_URL"); url != "" {
		p, err := Open(context.Background(), url)
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(p.Close)
		_, err = p.pool.Exec(context.Background(),
			`TRUNCATE captures, share_recipients, shares, devices, users RESTART IDENTITY CASCADE`)
		if err != nil {
			t.Fatal(err)
		}
		out["postgres"] = p
	}
	return out
}

var t0 = time.Date(2026, 10, 5, 12, 0, 0, 0, time.UTC)

func TestTheStores(t *testing.T) {
	for name, s := range stores(t) {
		t.Run(name, func(t *testing.T) { contract(t, s) })
	}
}

func TestTheStoresSync(t *testing.T) {
	for name, s := range stores(t) {
		t.Run(name, func(t *testing.T) { syncContract(t, s) })
	}
}

func contract(t *testing.T, s Store) {
	ctx := context.Background()
	must := func(err error) {
		t.Helper()
		if err != nil {
			t.Fatal(err)
		}
	}
	mark, err := s.AddUser(ctx, "mark")
	must(err)
	dana, err := s.AddUser(ctx, "dana")
	must(err)
	if _, err := s.AddUser(ctx, "mark"); !errors.Is(err, ErrExists) {
		t.Fatalf("a second mark: %v", err)
	}

	// Devices: a token works until revoked.
	d, err := s.AddDevice(ctx, mark.ID, "laptop", "hash1")
	must(err)
	if u, err := s.UserByToken(ctx, "hash1"); err != nil || u != mark {
		t.Fatalf("token: %v %v", u, err)
	}
	must(s.RevokeDevice(ctx, d.ID))
	if _, err := s.UserByToken(ctx, "hash1"); !errors.Is(err, ErrNotFound) {
		t.Fatalf("a revoked token: %v", err)
	}

	// Shares, with recipients, made in one go.
	expires := t0.Add(24 * time.Hour)
	share := Share{ID: "aaaaaaaaaa", Owner: mark.ID, SecretHash: "h", Size: 5, Type: "image/png",
		Created: t0, Expires: &expires, State: Pending, Public: true}
	must(s.CreateShare(ctx, share, []int64{dana.ID}))
	if err := s.CreateShare(ctx, share, nil); !errors.Is(err, ErrExists) {
		t.Fatalf("a taken id: %v", err)
	}

	// Moves happen only from the states given.
	if err := s.Move(ctx, share.ID, Live, Deleted); !errors.Is(err, ErrNotFound) {
		t.Fatalf("moved from the wrong state: %v", err)
	}
	must(s.Move(ctx, share.ID, Live, Pending))
	got, err := s.Share(ctx, share.ID)
	must(err)
	if got.State != Live || !got.Created.Equal(t0) {
		t.Fatalf("share: %+v", got)
	}

	owned, err := s.Shares(ctx, mark.ID)
	must(err)
	if len(owned) != 1 || !slices.Equal(owned[0].Recipients, []string{"dana"}) {
		t.Fatalf("owned: %+v", owned)
	}
	received, err := s.SharedWith(ctx, dana.ID, t0)
	must(err)
	if len(received) != 1 || received[0].From != "mark" {
		t.Fatalf("received: %+v", received)
	}

	// Access changes, leaving recipients alone unless given.
	must(s.SetAccess(ctx, share.ID, false, nil, nil))
	if r, _ := s.Recipients(ctx, share.ID); len(r) != 1 {
		t.Fatalf("recipients changed: %v", r)
	}
	none := []int64{}
	must(s.SetAccess(ctx, share.ID, false, &expires, &none))
	if r, _ := s.Recipients(ctx, share.ID); len(r) != 0 {
		t.Fatalf("recipients kept: %v", r)
	}

	// The sweeper takes the expired share and the abandoned one, and
	// leaves a fresh pending one.
	fresh := Share{ID: "bbbbbbbbbb", Owner: mark.ID, SecretHash: "h", Size: 5, Type: "image/png",
		Created: t0.Add(47 * time.Hour), State: Pending}
	old := fresh
	old.ID, old.Created = "cccccccccc", t0
	must(s.CreateShare(ctx, fresh, nil))
	must(s.CreateShare(ctx, old, nil))
	swept, err := s.Sweep(ctx, t0.Add(48*time.Hour), time.Hour, 10)
	must(err)
	var ids []string
	for _, sh := range swept {
		ids = append(ids, sh.ID)
	}
	slices.Sort(ids)
	if !slices.Equal(ids, []string{"aaaaaaaaaa", "cccccccccc"}) {
		t.Fatalf("swept %v", ids)
	}
	if got, _ := s.Share(ctx, "cccccccccc"); got.State != Deleted {
		t.Fatalf("swept but not deleted: %v", got.State)
	}
	if again, _ := s.Sweep(ctx, t0.Add(48*time.Hour), time.Hour, 10); len(again) != 0 {
		t.Fatalf("swept twice: %v", again)
	}
}
