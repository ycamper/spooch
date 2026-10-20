package api

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"slices"
	"sort"
	"time"

	"github.com/ycamper/spooch/service/internal/store"
)

// expiresIn is when a link lasts until, counting `days` from now: 0 for
// the default, -1 for never.
func (s *Server) expiresIn(days int) (*time.Time, error) {
	if days < -1 || days > 3650 {
		return nil, badRequest("expiry_days must be -1 (never) to 3650")
	}
	if days == -1 {
		return nil, nil
	}
	if days == 0 {
		days = DefaultExpiryDays
	}
	t := s.Now().AddDate(0, 0, days)
	return &t, nil
}

// recipients looks up accounts by name, leaving out the owner and
// repeats.
func (s *Server) recipients(ctx context.Context, owner store.User, names []string) ([]store.User, error) {
	if len(names) > 50 {
		return nil, badRequest("a share can go to at most 50 accounts")
	}
	var out []store.User
	for _, name := range names {
		if len(name) > 64 {
			return nil, badRequest("account names are at most 64 characters")
		}
		u, err := s.Store.UserByName(ctx, name)
		if errors.Is(err, store.ErrNotFound) {
			return nil, badRequest(fmt.Sprintf("there's no account named %q", name))
		} else if err != nil {
			return nil, err
		}
		if u.ID != owner.ID && !slices.Contains(out, u) {
			out = append(out, u)
		}
	}
	return out, nil
}

func ids(users []store.User) []int64 {
	out := make([]int64, len(users))
	for i, u := range users {
		out[i] = u.ID
	}
	return out
}

func names(users []store.User) []string {
	out := make([]string, len(users))
	for i, u := range users {
		out[i] = u.Name
	}
	sort.Strings(out)
	return out
}

type accessRequest struct {
	Public     *bool     `json:"public"`
	Recipients *[]string `json:"recipients"`
	// ExpiryDays counts from now, as when sharing.
	ExpiryDays *int `json:"expiry_days"`
}

// change updates who may view a share, and until when. Fields left out
// stay as they are; an expired share can't be brought back.
func (s *Server) change(w http.ResponseWriter, r *http.Request, u store.User) {
	share, ok := s.owned(w, r, u)
	if !ok {
		return
	}
	if s.expired(share) {
		fail(w, http.StatusGone, "that share has expired")
		return
	}
	var req accessRequest
	if !decode(w, r, 8<<10, &req) {
		return
	}
	ctx := r.Context()
	if req.Public != nil {
		share.Public = *req.Public
	}
	if req.ExpiryDays != nil {
		expires, err := s.expiresIn(*req.ExpiryDays)
		if err != nil {
			failWith(w, err)
			return
		}
		share.Expires = expires
	}
	var recipients []store.User
	var err error
	if req.Recipients != nil {
		recipients, err = s.recipients(ctx, u, *req.Recipients)
	} else {
		recipients, err = s.Store.Recipients(ctx, share.ID)
	}
	if err != nil {
		failWith(w, err)
		return
	}
	var set *[]int64
	if req.Recipients != nil {
		chosen := ids(recipients)
		set = &chosen
	}
	if err := s.Store.SetAccess(ctx, share.ID, share.Public, share.Expires, set); err != nil {
		failWith(w, err)
		return
	}
	reply(w, http.StatusOK, describe(share, names(recipients)))
}

type receivedJSON struct {
	ID      string     `json:"id"`
	From    string     `json:"from"`
	Size    int64      `json:"size"`
	Type    string     `json:"type"`
	Created time.Time  `json:"created"`
	Expires *time.Time `json:"expires,omitempty"`
}

// sharedWithMe lists what other accounts have shared with this one.
func (s *Server) sharedWithMe(w http.ResponseWriter, r *http.Request, u store.User) {
	got, err := s.Store.SharedWith(r.Context(), u.ID, s.Now())
	if err != nil {
		serverError(w, err)
		return
	}
	out := make([]receivedJSON, 0, len(got))
	for _, g := range got {
		out = append(out, receivedJSON{ID: g.ID, From: g.From, Size: g.Size, Type: g.Type, Created: g.Created, Expires: g.Expires})
	}
	reply(w, http.StatusOK, map[string]any{"shares": out})
}

// image serves a share's image to its owner or a recipient, by id: how
// the app shows shares that aren't public.
func (s *Server) image(w http.ResponseWriter, r *http.Request, u store.User) {
	ctx := r.Context()
	share, err := s.Store.Share(ctx, r.PathValue("id"))
	if err == nil && (share.State != store.Live || s.expired(share)) {
		err = store.ErrNotFound
	}
	if err == nil {
		var may bool
		if may, err = s.mayView(ctx, share, u); err == nil && !may {
			err = store.ErrNotFound
		}
	}
	if err != nil {
		failWith(w, err)
		return
	}
	s.serveImage(w, r, share, "private, no-store")
}

func (s *Server) mayView(ctx context.Context, share store.Share, u store.User) (bool, error) {
	if share.Owner == u.ID {
		return true, nil
	}
	users, err := s.Store.Recipients(ctx, share.ID)
	return slices.Contains(users, u), err
}
