package api

import (
	"context"
	"crypto/subtle"
	"errors"
	"net/http"
	"regexp"
	"strconv"
	"sync"
	"time"

	"github.com/ycamper/spooch/service/internal/password"
	"github.com/ycamper/spooch/service/internal/store"
	"github.com/ycamper/spooch/service/internal/token"
)

const (
	// Sign-ins to one account that may fail within `Lockout` before the
	// rest are refused until it passes.
	MaxFailures = 10
	Lockout     = 15 * time.Minute
	// A password's length, in bytes.
	MinPassword = 10
	MaxPassword = 256
)

// A name is lowercase letters, digits, dots, dashes and underscores,
// 3 to 32 of them, starting with a letter or digit.
var namePattern = regexp.MustCompile(`^[a-z0-9][a-z0-9._-]{2,31}$`)

type signUp struct {
	Name     string `json:"name"`
	Password string `json:"password"`
	// Code is the sign-up code the service was set up with.
	Code string `json:"code"`
	// Device names the device signing in, like "laptop".
	Device string `json:"device"`
}

type signIn struct {
	Name     string `json:"name"`
	Password string `json:"password"`
	Device   string `json:"device"`
}

func checkPassword(p string) error {
	if len(p) < MinPassword || len(p) > MaxPassword {
		return badRequest("a password is 10 to 256 characters")
	}
	return nil
}

func checkDevice(label string) error {
	if label == "" || len(label) > 64 {
		return badRequest("device must name the device, in up to 64 characters")
	}
	return nil
}

// createAccount makes an account and signs its first device in.
func (s *Server) createAccount(w http.ResponseWriter, r *http.Request) {
	var req signUp
	if !decode(w, r, 4<<10, &req) {
		return
	}
	if s.SignupCode == "" {
		fail(w, http.StatusForbidden, "sign-ups are closed")
		return
	}
	if subtle.ConstantTimeCompare([]byte(req.Code), []byte(s.SignupCode)) != 1 {
		fail(w, http.StatusForbidden, "that sign-up code isn't right")
		return
	}
	var err error
	switch {
	case !namePattern.MatchString(req.Name):
		err = badRequest("a name is 3 to 32 lowercase letters, digits, dots, dashes or underscores")
	default:
		if err = checkPassword(req.Password); err == nil {
			err = checkDevice(req.Device)
		}
	}
	if err != nil {
		failWith(w, err)
		return
	}
	hash, err := password.Hash(req.Password)
	if err != nil {
		serverError(w, err)
		return
	}
	ctx := r.Context()
	u, err := s.Store.AddAccount(ctx, req.Name, hash)
	if errors.Is(err, store.ErrExists) {
		fail(w, http.StatusConflict, "that name is taken")
		return
	} else if err != nil {
		serverError(w, err)
		return
	}
	s.signedIn(ctx, w, http.StatusCreated, u, req.Device)
}

// signIn checks a name and password, and gives the device a token.
func (s *Server) signIn(w http.ResponseWriter, r *http.Request) {
	var req signIn
	if !decode(w, r, 4<<10, &req) {
		return
	}
	if err := checkDevice(req.Device); err != nil {
		failWith(w, err)
		return
	}
	if !s.throttle.allowed(req.Name, s.Now()) {
		fail(w, http.StatusTooManyRequests, "too many wrong passwords; try again in a while")
		return
	}
	ctx := r.Context()
	u, hash, err := s.Store.Password(ctx, req.Name)
	if err != nil && !errors.Is(err, store.ErrNotFound) {
		serverError(w, err)
		return
	}
	ok := false
	if hash == "" {
		// No such account, or none with a password: as slow as a wrong one.
		password.Waste(req.Password)
	} else {
		ok = password.Check(req.Password, hash)
	}
	if !ok {
		s.throttle.failed(req.Name, s.Now())
		fail(w, http.StatusUnauthorized, "wrong name or password")
		return
	}
	s.throttle.forget(req.Name)
	s.signedIn(ctx, w, http.StatusOK, u, req.Device)
}

// signedIn gives a device of `u` a token, shown this once.
func (s *Server) signedIn(ctx context.Context, w http.ResponseWriter, status int, u store.User, label string) {
	device := token.NewDevice()
	d, err := s.Store.AddDevice(ctx, u.ID, label, token.Hash(device))
	if err != nil {
		serverError(w, err)
		return
	}
	reply(w, status, map[string]any{"name": u.Name, "token": device, "device": d.ID})
}

type passwordChange struct {
	// Current is the password now; an account without one leaves it out.
	Current  string `json:"current"`
	Password string `json:"password"`
}

// setPassword sets or changes the signed-in account's password.
func (s *Server) setPassword(w http.ResponseWriter, r *http.Request, u store.User) {
	var req passwordChange
	if !decode(w, r, 4<<10, &req) {
		return
	}
	if err := checkPassword(req.Password); err != nil {
		failWith(w, err)
		return
	}
	ctx := r.Context()
	_, now, err := s.Store.Password(ctx, u.Name)
	if err != nil {
		serverError(w, err)
		return
	}
	if now != "" && !password.Check(req.Current, now) {
		fail(w, http.StatusForbidden, "the current password isn't right")
		return
	}
	hash, err := password.Hash(req.Password)
	if err == nil {
		err = s.Store.SetPassword(ctx, u.ID, hash)
	}
	if err != nil {
		serverError(w, err)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

func (s *Server) devices(w http.ResponseWriter, r *http.Request, u store.User) {
	devices, err := s.Store.UserDevices(r.Context(), u.ID)
	if err != nil {
		serverError(w, err)
		return
	}
	out := make([]map[string]any, 0, len(devices))
	for _, d := range devices {
		out = append(out, map[string]any{
			"id": d.ID, "label": d.Label, "created": d.Created, "last_used": d.LastUsed,
		})
	}
	reply(w, http.StatusOK, map[string]any{"devices": out})
}

// signOut revokes one of the account's devices (this one, or another).
func (s *Server) signOut(w http.ResponseWriter, r *http.Request, u store.User) {
	id, err := strconv.ParseInt(r.PathValue("id"), 10, 64)
	if err == nil {
		err = s.Store.RevokeUserDevice(r.Context(), u.ID, id)
	}
	if err != nil {
		fail(w, http.StatusNotFound, "no such device")
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

// throttle counts failed sign-ins per account name.
type throttle struct {
	mu    sync.Mutex
	tries map[string]tries
}

type tries struct {
	count int
	since time.Time
}

func (t *throttle) allowed(name string, now time.Time) bool {
	t.mu.Lock()
	defer t.mu.Unlock()
	n, ok := t.tries[name]
	return !ok || n.count < MaxFailures || now.Sub(n.since) >= Lockout
}

func (t *throttle) failed(name string, now time.Time) {
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.tries == nil {
		t.tries = map[string]tries{}
	}
	// Names nobody signs in as again are let go of, so guesses at made-up
	// names can't fill memory.
	if len(t.tries) > 10_000 {
		for k, v := range t.tries {
			if now.Sub(v.since) >= Lockout {
				delete(t.tries, k)
			}
		}
	}
	n := t.tries[name]
	if now.Sub(n.since) >= Lockout {
		n = tries{since: now}
	}
	n.count++
	t.tries[name] = n
}

func (t *throttle) forget(name string) {
	t.mu.Lock()
	defer t.mu.Unlock()
	delete(t.tries, name)
}
