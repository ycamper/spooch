package store

import (
	"context"
	"sort"
	"time"
)

func (m *Memory) AddAccount(ctx context.Context, name, passwordHash string) (User, error) {
	u, err := m.AddUser(ctx, name)
	if err != nil {
		return u, err
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	m.passwords[u.ID] = passwordHash
	return u, nil
}

func (m *Memory) Password(ctx context.Context, name string) (User, string, error) {
	u, err := m.UserByName(ctx, name)
	if err != nil {
		return u, "", err
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	return u, m.passwords[u.ID], nil
}

func (m *Memory) SetPassword(_ context.Context, user int64, hash string) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if _, ok := m.user(user); !ok {
		return ErrNotFound
	}
	m.passwords[user] = hash
	return nil
}

func (m *Memory) UserDevices(_ context.Context, user int64) ([]Device, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	var out []Device
	for _, d := range m.devices {
		if m.owners[d.ID] == user && !d.Revoked {
			out = append(out, d)
		}
	}
	return out, nil
}

func (m *Memory) RevokeUserDevice(_ context.Context, user, id int64) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if id < 1 || int(id) > len(m.devices) || m.owners[id] != user || m.devices[id-1].Revoked {
		return ErrNotFound
	}
	m.devices[id-1].Revoked = true
	return nil
}

// next is the next number from the change counter. The caller holds the
// lock.
func (m *Memory) next() int64 {
	m.changes++
	return m.changes
}

func (m *Memory) BeginCapture(_ context.Context, owner int64, id string, base int64, taken time.Time, sizes Sizes, now time.Time) (int64, int64, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.captures[owner] == nil {
		m.captures[owner] = map[string]Capture{}
	}
	c, ok := m.captures[owner][id]
	switch {
	case !ok && base != 0:
		return 0, 0, ErrConflict
	case !ok:
		c = Capture{ID: id, Owner: owner, Taken: taken, Updated: now}
	case c.Gone:
		return 0, 0, ErrGone
	case c.Trashed != nil:
		return 0, 0, ErrTrashed
	case c.Version != base:
		return 0, 0, ErrConflict
	}
	replaced := c.Pending
	c.Pending, c.PendingSizes, c.Begun = m.next(), sizes, &now
	m.captures[owner][id] = c
	return c.Pending, replaced, nil
}

func (m *Memory) FinishCapture(_ context.Context, owner int64, id string, version int64, now time.Time) (Capture, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	c, ok := m.captures[owner][id]
	if !ok || c.Pending != version || version == 0 || c.Gone || c.Trashed != nil {
		return Capture{}, ErrNotFound
	}
	before := c
	c.Version, c.Sizes, c.Updated, c.Seq = c.Pending, c.PendingSizes, now, m.next()
	c.Pending, c.PendingSizes, c.Begun = 0, Sizes{}, nil
	m.captures[owner][id] = c
	return before, nil
}

func (m *Memory) Capture(_ context.Context, owner int64, id string) (Capture, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	c, ok := m.captures[owner][id]
	if !ok {
		return Capture{}, ErrNotFound
	}
	return c, nil
}

func (m *Memory) Changes(_ context.Context, owner int64, since int64, limit int) ([]Capture, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	var out []Capture
	for _, c := range m.captures[owner] {
		if c.Seq > since && c.Version != 0 {
			out = append(out, c)
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Seq < out[j].Seq })
	if len(out) > limit {
		out = out[:limit]
	}
	return out, nil
}

// change applies `f` to a capture that's live (or, with `trashed`, in the
// trash), moving it on in the changes. The caller holds the lock.
func (m *Memory) change(owner int64, id string, trashed bool, f func(*Capture)) error {
	c, ok := m.captures[owner][id]
	if !ok || c.Version == 0 || c.Gone || (c.Trashed != nil) != trashed {
		return ErrNotFound
	}
	f(&c)
	c.Seq = m.next()
	m.captures[owner][id] = c
	return nil
}

func (m *Memory) TrashCapture(_ context.Context, owner int64, id string, now time.Time) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.change(owner, id, false, func(c *Capture) { c.Trashed = &now })
}

func (m *Memory) RestoreCapture(_ context.Context, owner int64, id string) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.change(owner, id, true, func(c *Capture) { c.Trashed = nil })
}

func (m *Memory) PurgeCapture(_ context.Context, owner int64, id string) (Capture, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.purge(owner, id)
}

// purge is PurgeCapture; the caller holds the lock.
func (m *Memory) purge(owner int64, id string) (Capture, error) {
	c, ok := m.captures[owner][id]
	if !ok || c.Version == 0 || c.Gone {
		return Capture{}, ErrNotFound
	}
	before := c
	c.Gone, c.Seq = true, m.next()
	c.Pending, c.PendingSizes, c.Begun = 0, Sizes{}, nil
	m.captures[owner][id] = c
	return before, nil
}

func (m *Memory) Usage(_ context.Context, owner int64) (int64, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	var total int64
	for _, c := range m.captures[owner] {
		if !c.Gone {
			total += c.Sizes.Total()
		}
		total += c.PendingSizes.Total()
	}
	return total, nil
}

func (m *Memory) SweepCaptures(_ context.Context, trashedBefore, begunBefore time.Time, limit int) ([]Capture, []Capture, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	var purged, dropped []Capture
	for owner, byID := range m.captures {
		for id, c := range byID {
			switch {
			case len(purged) < limit && !c.Gone && c.Trashed != nil && c.Trashed.Before(trashedBefore):
				was, err := m.purge(owner, id)
				if err == nil {
					purged = append(purged, was)
				}
			case len(dropped) < limit && c.Begun != nil && c.Begun.Before(begunBefore):
				dropped = append(dropped, c)
				if c.Version == 0 {
					delete(byID, id)
				} else {
					c.Pending, c.PendingSizes, c.Begun = 0, Sizes{}, nil
					byID[id] = c
				}
			}
		}
	}
	return purged, dropped, nil
}
