package api

import (
	"bytes"
	"context"
	"errors"
	"io"
	"log"
	"net/http"
	"time"

	"github.com/ycamper/spooch/service/internal/storage"
	"github.com/ycamper/spooch/service/internal/store"
	"github.com/ycamper/spooch/service/internal/token"
)

type createRequest struct {
	ID         string `json:"id"`
	SecretHash string `json:"secret_hash"`
	Size       int64  `json:"size"`
	Type       string `json:"type"`
	// ExpiryDays: 0 for the default, -1 for never.
	ExpiryDays int `json:"expiry_days"`
	// Public: anyone with the link may view it (the default).
	Public *bool `json:"public"`
	// Recipients are the accounts it's shared with, by name.
	Recipients []string `json:"recipients"`
}

type shareJSON struct {
	ID         string     `json:"id"`
	Size       int64      `json:"size"`
	Type       string     `json:"type"`
	Created    time.Time  `json:"created"`
	Expires    *time.Time `json:"expires,omitempty"`
	Public     bool       `json:"public"`
	Recipients []string   `json:"recipients"`
}

// describe is how the API shows a share to its owner.
func describe(sh store.Share, recipients []string) shareJSON {
	if recipients == nil {
		recipients = []string{}
	}
	return shareJSON{
		ID: sh.ID, Size: sh.Size, Type: sh.Type, Created: sh.Created,
		Expires: sh.Expires, Public: sh.Public, Recipients: recipients,
	}
}

func (s *Server) create(w http.ResponseWriter, r *http.Request, u store.User) {
	var req createRequest
	if !decode(w, r, 4<<10, &req) {
		return
	}
	ctx := r.Context()
	share, recipients, err := s.newShare(ctx, u, req)
	if err != nil {
		failWith(w, err)
		return
	}
	if err := s.Store.CreateShare(ctx, share, ids(recipients)); errors.Is(err, store.ErrExists) {
		fail(w, http.StatusConflict, "that id is taken; pick another")
		return
	} else if err != nil {
		serverError(w, err)
		return
	}
	upload, err := s.Storage.PresignPut(ctx, uploadKey(share.ID), share.Type, share.Size, UploadTTL)
	if err != nil {
		serverError(w, err)
		return
	}
	reply(w, http.StatusCreated, map[string]any{
		"upload_url":     upload,
		"upload_expires": s.Now().Add(UploadTTL),
		"content_type":   share.Type,
		"expires":        share.Expires,
		"public":         share.Public,
	})
}

// newShare checks a create request and makes the share it asks for.
func (s *Server) newShare(ctx context.Context, u store.User, req createRequest) (store.Share, []store.User, error) {
	switch {
	case !token.IsBase62(req.ID, token.IDLen):
		return store.Share{}, nil, badRequest("id must be 10 base62 characters")
	case !token.IsHash(req.SecretHash):
		return store.Share{}, nil, badRequest("secret_hash must be a lowercase hex SHA-256")
	case types[req.Type] == "":
		return store.Share{}, nil, badRequest("type must be image/png or image/jpeg")
	case req.Size <= 0 || req.Size > MaxSize:
		return store.Share{}, nil, badRequest("size must be 1 byte to 25 MB")
	}
	expires, err := s.expiresIn(req.ExpiryDays)
	if err != nil {
		return store.Share{}, nil, err
	}
	recipients, err := s.recipients(ctx, u, req.Recipients)
	if err != nil {
		return store.Share{}, nil, err
	}
	share := store.Share{
		ID: req.ID, Owner: u.ID, SecretHash: req.SecretHash, Size: req.Size, Type: req.Type,
		Created: s.Now(), Expires: expires, State: store.Pending, Public: req.Public == nil || *req.Public,
	}
	return share, recipients, nil
}

// owned is share `id` if it belongs to `u` and isn't deleted; otherwise
// it replies and returns false.
func (s *Server) owned(w http.ResponseWriter, r *http.Request, u store.User) (store.Share, bool) {
	share, err := s.Store.Share(r.Context(), r.PathValue("id"))
	if err == nil && (share.Owner != u.ID || share.State == store.Deleted) {
		err = store.ErrNotFound
	}
	if err != nil {
		failWith(w, err)
		return store.Share{}, false
	}
	return share, true
}

// Magic numbers for the types a share may hold.
var magic = map[string][]byte{
	"image/png":  {0x89, 'P', 'N', 'G', '\r', '\n', 0x1a, '\n'},
	"image/jpeg": {0xff, 0xd8, 0xff},
}

func (s *Server) finalize(w http.ResponseWriter, r *http.Request, u store.User) {
	share, ok := s.owned(w, r, u)
	if !ok {
		return
	}
	live := map[string]any{"state": store.Live}
	if share.State == store.Live {
		reply(w, http.StatusOK, live)
		return
	}
	ctx := r.Context()
	good, err := s.verify(ctx, share)
	switch {
	case errors.Is(err, storage.ErrNotFound):
		fail(w, http.StatusConflict, "nothing has been uploaded yet")
		return
	case err != nil:
		serverError(w, err)
		return
	case !good:
		if err := s.Store.Move(ctx, share.ID, store.Deleted, store.Pending); err == nil {
			s.purge(ctx, share.ID)
		}
		fail(w, http.StatusUnprocessableEntity, "the upload isn't the image that was declared")
		return
	}
	if err := s.Storage.Copy(ctx, uploadKey(share.ID), shareKey(share.ID)); err != nil {
		serverError(w, err)
		return
	}
	// Deleted meanwhile: it stays deleted.
	if err := s.Store.Move(ctx, share.ID, store.Live, store.Pending); err != nil {
		s.purge(ctx, share.ID)
		failWith(w, err)
		return
	}
	if err := s.Storage.Delete(ctx, uploadKey(share.ID)); err != nil {
		log.Printf("finalize %s: removing the upload: %v", share.ID, err)
	}
	reply(w, http.StatusOK, live)
}

// verify checks the upload is the image the share declared: its size,
// and how it starts.
func (s *Server) verify(ctx context.Context, share store.Share) (bool, error) {
	size, err := s.Storage.Size(ctx, uploadKey(share.ID))
	if err != nil || size != share.Size {
		return false, err
	}
	f, err := s.Storage.Open(ctx, uploadKey(share.ID))
	if err != nil {
		return false, err
	}
	defer f.Close()
	want := magic[share.Type]
	head := make([]byte, len(want))
	if _, err := io.ReadFull(f, head); errors.Is(err, io.EOF) || errors.Is(err, io.ErrUnexpectedEOF) {
		return false, nil
	} else if err != nil {
		return false, err
	}
	return bytes.Equal(head, want), nil
}

func (s *Server) list(w http.ResponseWriter, r *http.Request, u store.User) {
	shares, err := s.Store.Shares(r.Context(), u.ID)
	if err != nil {
		serverError(w, err)
		return
	}
	out := make([]shareJSON, 0, len(shares))
	for _, sh := range shares {
		out = append(out, describe(sh.Share, sh.Recipients))
	}
	reply(w, http.StatusOK, map[string]any{"shares": out})
}

func (s *Server) remove(w http.ResponseWriter, r *http.Request, u store.User) {
	share, ok := s.owned(w, r, u)
	if !ok {
		return
	}
	if err := s.Store.Move(r.Context(), share.ID, store.Deleted, store.Pending, store.Live); err != nil {
		failWith(w, err)
		return
	}
	s.purge(r.Context(), share.ID)
	w.WriteHeader(http.StatusNoContent)
}

// purge removes a deleted share's objects. The record already says
// deleted, so a failure is only logged.
func (s *Server) purge(ctx context.Context, id string) {
	for _, key := range []string{uploadKey(id), shareKey(id)} {
		if err := s.Storage.Delete(ctx, key); err != nil {
			log.Printf("removing %s: %v", key, err)
		}
	}
}

// Sweep clears out expired and abandoned shares, and captures a month in
// the trash or never finished, a batch at a time. Returns how many shares
// and captures.
func (s *Server) Sweep(ctx context.Context) (int, int, error) {
	shares, err := s.sweepShares(ctx)
	if err != nil {
		return shares, 0, err
	}
	purged, dropped, err := s.sweepCaptures(ctx)
	return shares, purged + dropped, err
}

func (s *Server) sweepShares(ctx context.Context) (int, error) {
	total := 0
	for {
		gone, err := s.Store.Sweep(ctx, s.Now(), Grace, SweepBatch)
		if err != nil {
			return total, err
		}
		for _, share := range gone {
			s.purge(ctx, share.ID)
		}
		total += len(gone)
		if len(gone) < SweepBatch {
			return total, nil
		}
	}
}
