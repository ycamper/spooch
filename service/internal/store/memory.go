package store

import (
	"context"
	"slices"
	"sort"
	"sync"
	"time"
)

// Memory is a Store in memory, for tests.
type Memory struct {
	mu      sync.Mutex
	users   []User
	devices []Device
	tokens  map[string]int64 // token hash to device id
	owners  map[int64]int64  // device id to user id
	shares  map[string]Share
	// recipients: share id to user ids.
	recipients map[string][]int64
	passwords  map[int64]string
	// captures: by owner, then id.
	captures map[int64]map[string]Capture
	changes  int64
}

func NewMemory() *Memory {
	return &Memory{
		tokens:     map[string]int64{},
		owners:     map[int64]int64{},
		shares:     map[string]Share{},
		recipients: map[string][]int64{},
		passwords:  map[int64]string{},
		captures:   map[int64]map[string]Capture{},
	}
}

// user is user `id`, if there is one. The caller holds the lock.
func (m *Memory) user(id int64) (User, bool) {
	if id < 1 || int(id) > len(m.users) {
		return User{}, false
	}
	return m.users[id-1], true
}

func (m *Memory) AddUser(_ context.Context, name string) (User, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	for _, u := range m.users {
		if u.Name == name {
			return User{}, ErrExists
		}
	}
	u := User{ID: int64(len(m.users) + 1), Name: name}
	m.users = append(m.users, u)
	return u, nil
}

func (m *Memory) UserByName(_ context.Context, name string) (User, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	for _, u := range m.users {
		if u.Name == name {
			return u, nil
		}
	}
	return User{}, ErrNotFound
}

func (m *Memory) AddDevice(_ context.Context, user int64, label, tokenHash string) (Device, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	u, ok := m.user(user)
	if !ok {
		return Device{}, ErrNotFound
	}
	if _, ok := m.tokens[tokenHash]; ok {
		return Device{}, ErrExists
	}
	d := Device{ID: int64(len(m.devices) + 1), User: u.Name, Label: label, Created: time.Now()}
	m.devices = append(m.devices, d)
	m.tokens[tokenHash] = d.ID
	m.owners[d.ID] = user
	return d, nil
}

func (m *Memory) RevokeDevice(_ context.Context, id int64) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if id < 1 || int(id) > len(m.devices) {
		return ErrNotFound
	}
	m.devices[id-1].Revoked = true
	return nil
}

func (m *Memory) Devices(context.Context) ([]Device, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	return slices.Clone(m.devices), nil
}

func (m *Memory) UserByToken(_ context.Context, tokenHash string) (User, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	id, ok := m.tokens[tokenHash]
	if !ok || m.devices[id-1].Revoked {
		return User{}, ErrNotFound
	}
	now := time.Now()
	m.devices[id-1].LastUsed = &now
	u, _ := m.user(m.owners[id])
	return u, nil
}

func (m *Memory) CreateShare(_ context.Context, s Share, recipients []int64) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if _, ok := m.shares[s.ID]; ok {
		return ErrExists
	}
	m.shares[s.ID] = s
	m.recipients[s.ID] = slices.Clone(recipients)
	return nil
}

func (m *Memory) Share(_ context.Context, id string) (Share, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	s, ok := m.shares[id]
	if !ok {
		return Share{}, ErrNotFound
	}
	return s, nil
}

func (m *Memory) Move(_ context.Context, id string, to State, from ...State) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	s, ok := m.shares[id]
	if !ok || !slices.Contains(from, s.State) {
		return ErrNotFound
	}
	s.State = to
	m.shares[id] = s
	return nil
}

// names are the names of users `ids`, sorted. The caller holds the lock.
func (m *Memory) names(ids []int64) []User {
	var out []User
	for _, id := range ids {
		if u, ok := m.user(id); ok {
			out = append(out, u)
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Name < out[j].Name })
	return out
}

func (m *Memory) Shares(_ context.Context, owner int64) ([]Owned, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	out := []Owned{}
	for _, s := range m.shares {
		if s.Owner != owner || s.State != Live {
			continue
		}
		names := []string{}
		for _, u := range m.names(m.recipients[s.ID]) {
			names = append(names, u.Name)
		}
		out = append(out, Owned{Share: s, Recipients: names})
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Created.After(out[j].Created) })
	return out, nil
}

func (m *Memory) SetAccess(_ context.Context, id string, public bool, expires *time.Time, recipients *[]int64) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	s, ok := m.shares[id]
	if !ok {
		return ErrNotFound
	}
	s.Public, s.Expires = public, expires
	m.shares[id] = s
	if recipients != nil {
		m.recipients[id] = slices.Clone(*recipients)
	}
	return nil
}

func (m *Memory) Recipients(_ context.Context, id string) ([]User, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.names(m.recipients[id]), nil
}

func (m *Memory) SharedWith(_ context.Context, user int64, now time.Time) ([]Received, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	var out []Received
	for id, users := range m.recipients {
		s := m.shares[id]
		live := s.State == Live && (s.Expires == nil || s.Expires.After(now))
		if live && slices.Contains(users, user) {
			owner, _ := m.user(s.Owner)
			out = append(out, Received{Share: s, From: owner.Name})
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Created.After(out[j].Created) })
	return out, nil
}

func (m *Memory) Sweep(_ context.Context, now time.Time, grace time.Duration, limit int) ([]Share, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	var out []Share
	for id, s := range m.shares {
		expired := s.State == Live && s.Expires != nil && s.Expires.Before(now)
		abandoned := s.State == Pending && s.Created.Before(now.Add(-grace))
		if (expired || abandoned) && len(out) < limit {
			s.State = Deleted
			m.shares[id] = s
			out = append(out, s)
		}
	}
	return out, nil
}
