// Package api is the service's HTTP side: the app's share API, and the
// pages and images a share link opens.
//
// The app picks a share's link itself (an id and a secret, see the token
// package) so it's on the clipboard at once, sends the id with a hash of
// the secret, uploads to a presigned URL, then finalizes. Only the hash
// is stored; the secret comes with each view and is never logged.
package api

import (
	"errors"
	"net/http"
	"strings"
	"time"

	"github.com/ycamper/spooch/service/internal/storage"
	"github.com/ycamper/spooch/service/internal/store"
	"github.com/ycamper/spooch/service/internal/token"
)

const (
	// MaxSize is the largest image a share may hold.
	MaxSize = 25 << 20
	// UploadTTL is how long a presigned upload URL works.
	UploadTTL = 5 * time.Minute
	// DefaultExpiryDays is how long a link lasts unless the app asks.
	DefaultExpiryDays = 30
	// Grace is how long a share may wait for its upload before it's
	// cleared out.
	Grace = time.Hour
	// SweepBatch is how many shares a sweep clears at a time.
	SweepBatch = 500
)

// types maps the image types a share may hold to their file extension.
var types = map[string]string{"image/png": "png", "image/jpeg": "jpg"}

// Server serves the API and the links.
type Server struct {
	Store   store.Store
	Storage storage.Storage
	// PublicURL is where links point, like "https://img.spooch.app".
	PublicURL string
	// Now is the clock; tests set it.
	Now func() time.Time
	// SignupCode is what creating an account takes; "" closes sign-ups.
	SignupCode string
	// Quota is how many bytes of captures an account may keep.
	Quota int64

	throttle throttle
}

func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /v1/me", s.authed(s.me))
	mux.HandleFunc("POST /v1/shares", s.authed(s.create))
	mux.HandleFunc("GET /v1/shares", s.authed(s.list))
	mux.HandleFunc("POST /v1/shares/{id}/finalize", s.authed(s.finalize))
	mux.HandleFunc("PATCH /v1/shares/{id}", s.authed(s.change))
	mux.HandleFunc("DELETE /v1/shares/{id}", s.authed(s.remove))
	mux.HandleFunc("GET /v1/shares/{id}/image", s.authed(s.image))
	mux.HandleFunc("GET /v1/shared-with-me", s.authed(s.sharedWithMe))
	mux.HandleFunc("POST /v1/accounts", s.createAccount)
	mux.HandleFunc("POST /v1/sign-in", s.signIn)
	mux.HandleFunc("PUT /v1/password", s.authed(s.setPassword))
	mux.HandleFunc("GET /v1/devices", s.authed(s.devices))
	mux.HandleFunc("DELETE /v1/devices/{id}", s.authed(s.signOut))
	mux.HandleFunc("GET /v1/captures", s.authed(s.changes))
	mux.HandleFunc("PUT /v1/captures/{id}", s.authed(s.beginCapture))
	mux.HandleFunc("POST /v1/captures/{id}/finish", s.authed(s.finishCapture))
	mux.HandleFunc("GET /v1/captures/{id}/{part}", s.authed(s.capturePart))
	mux.HandleFunc("DELETE /v1/captures/{id}", s.authed(s.trashCapture))
	mux.HandleFunc("POST /v1/captures/{id}/restore", s.authed(s.restoreCapture))
	mux.HandleFunc("GET /{link}", s.view)
	mux.HandleFunc("GET /{$}", s.home)
	return headers(mux)
}

// headers sets what every response says: don't sniff types, keep it out
// of search, and don't leak links through the Referer header.
func headers(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		h := w.Header()
		h.Set("X-Content-Type-Options", "nosniff")
		h.Set("X-Robots-Tag", "noindex, nofollow")
		h.Set("Referrer-Policy", "no-referrer")
		next.ServeHTTP(w, r)
	})
}

// Where a share's image lives: uploads land in one place, and finalize
// copies a checked upload to the other, so a late upload can't replace a
// live image.
func uploadKey(id string) string { return "uploads/" + id }
func shareKey(id string) string  { return "shares/" + id }

// authed runs `h` for the user whose device token the request carries.
func (s *Server) authed(h func(http.ResponseWriter, *http.Request, store.User)) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		bearer, ok := strings.CutPrefix(r.Header.Get("Authorization"), "Bearer ")
		if !ok || !strings.HasPrefix(bearer, token.DevicePrefix) {
			fail(w, http.StatusUnauthorized, "a device token is needed")
			return
		}
		u, err := s.Store.UserByToken(r.Context(), token.Hash(bearer))
		if errors.Is(err, store.ErrNotFound) {
			fail(w, http.StatusUnauthorized, "that device token isn't valid")
			return
		} else if err != nil {
			serverError(w, err)
			return
		}
		h(w, r, u)
	}
}

// me names the token's account, and where links point, so the app can
// make links before the service hears of them.
func (s *Server) me(w http.ResponseWriter, _ *http.Request, u store.User) {
	reply(w, http.StatusOK, map[string]any{"name": u.Name, "links": s.PublicURL})
}

func (s *Server) expired(share store.Share) bool {
	return share.Expires != nil && share.Expires.Before(s.Now())
}
