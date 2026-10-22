package api

import (
	"context"
	"errors"
	"fmt"
	"log"
	"net/http"
	"strconv"
	"time"

	"github.com/ycamper/spooch/service/internal/storage"
	"github.com/ycamper/spooch/service/internal/store"
	"github.com/ycamper/spooch/service/internal/token"
)

const (
	// CaptureIDLen is how long the ids the app gives captures are.
	CaptureIDLen = 16
	// TrashDays is how long a deleted capture can be restored.
	TrashDays = 30
	// GetTTL is how long a download URL works.
	GetTTL = time.Hour
	// MaxChanges is the most changes one request lists.
	MaxChanges = 500
)

// A capture's parts: what each is called, its type, its largest size,
// and how it starts.
type part struct {
	name, contentType string
	max               int64
}

var parts = []part{
	{"doc", "application/json", 4 << 20},
	{"source", "image/png", 64 << 20},
	{"thumb", "image/png", 2 << 20},
}

func partSize(s store.Sizes, name string) int64 {
	switch name {
	case "doc":
		return s.Doc
	case "source":
		return s.Source
	default:
		return s.Thumb
	}
}

// captureKey is where version `version` of a capture's part `name` lives.
func captureKey(owner int64, id string, version int64, name string) string {
	return fmt.Sprintf("captures/%d/%s/%d/%s", owner, id, version, name)
}

type captureJSON struct {
	ID      string     `json:"id"`
	Version int64      `json:"version"`
	Seq     int64      `json:"seq"`
	Taken   time.Time  `json:"taken"`
	Updated time.Time  `json:"updated"`
	Size    int64      `json:"size"`
	Trashed *time.Time `json:"trashed,omitempty"`
	Gone    bool       `json:"gone,omitempty"`
	// ThumbURL fetches its thumbnail for an hour, for previews.
	ThumbURL string `json:"thumb_url,omitempty"`
}

type beginRequest struct {
	// Base is the version the new one replaces, 0 for a new capture.
	Base   int64     `json:"base"`
	Taken  time.Time `json:"taken"`
	Doc    int64     `json:"doc_size"`
	Source int64     `json:"source_size"`
	Thumb  int64     `json:"thumb_size"`
}

// captureFailed replies to the store's capture errors.
func captureFailed(w http.ResponseWriter, err error) {
	switch {
	case errors.Is(err, store.ErrNotFound):
		fail(w, http.StatusNotFound, "no such capture")
	case errors.Is(err, store.ErrTrashed):
		fail(w, http.StatusConflict, "it's in the trash; restore it first")
	case errors.Is(err, store.ErrGone):
		fail(w, http.StatusGone, "it was deleted for good")
	default:
		failWith(w, err)
	}
}

// beginCapture starts uploading a new version: the reply has a presigned
// PUT for each part.
func (s *Server) beginCapture(w http.ResponseWriter, r *http.Request, u store.User) {
	id := r.PathValue("id")
	var req beginRequest
	if !decode(w, r, 4<<10, &req) {
		return
	}
	sizes := store.Sizes{Doc: req.Doc, Source: req.Source, Thumb: req.Thumb}
	if !token.IsBase62(id, CaptureIDLen) {
		failWith(w, badRequest("a capture's id is 16 base62 characters"))
		return
	}
	for _, p := range parts {
		if n := partSize(sizes, p.name); n <= 0 || n > p.max {
			failWith(w, badRequest(fmt.Sprintf("%s_size must be 1 byte to %d MB", p.name, p.max>>20)))
			return
		}
	}
	if req.Taken.IsZero() || req.Base < 0 {
		failWith(w, badRequest("taken must be when it was taken, and base 0 or a version"))
		return
	}
	ctx := r.Context()
	used, err := s.Store.Usage(ctx, u.ID)
	if err != nil {
		serverError(w, err)
		return
	}
	if used+sizes.Total() > s.Quota {
		fail(w, http.StatusInsufficientStorage, "the account is full")
		return
	}
	version, replaced, err := s.Store.BeginCapture(ctx, u.ID, id, req.Base, req.Taken, sizes, s.Now())
	if errors.Is(err, store.ErrConflict) {
		current := int64(0)
		if c, err := s.Store.Capture(ctx, u.ID, id); err == nil {
			current = c.Version
		}
		reply(w, http.StatusConflict, map[string]any{
			"error": "it changed on another device meanwhile", "version": current,
		})
		return
	} else if err != nil {
		captureFailed(w, err)
		return
	}
	if replaced != 0 {
		s.removeVersion(ctx, u.ID, id, replaced)
	}
	uploads, types := map[string]string{}, map[string]string{}
	for _, p := range parts {
		url, err := s.Storage.PresignPut(ctx, captureKey(u.ID, id, version, p.name),
			p.contentType, partSize(sizes, p.name), UploadTTL)
		if err != nil {
			serverError(w, err)
			return
		}
		uploads[p.name], types[p.name] = url, p.contentType
	}
	reply(w, http.StatusOK, map[string]any{
		"version":        version,
		"uploads":        uploads,
		"types":          types,
		"upload_expires": s.Now().Add(UploadTTL),
	})
}

type finishRequest struct {
	Version int64 `json:"version"`
}

// finishCapture checks a version's uploads and makes it live.
func (s *Server) finishCapture(w http.ResponseWriter, r *http.Request, u store.User) {
	id := r.PathValue("id")
	var req finishRequest
	if !decode(w, r, 1<<10, &req) {
		return
	}
	ctx := r.Context()
	c, err := s.Store.Capture(ctx, u.ID, id)
	if err != nil {
		captureFailed(w, err)
		return
	}
	if c.Pending != req.Version || req.Version == 0 {
		fail(w, http.StatusConflict, "that isn't the version being uploaded")
		return
	}
	for _, p := range parts {
		problem, err := s.checkUpload(ctx, captureKey(u.ID, id, req.Version, p.name), partSize(c.PendingSizes, p.name), p.contentType)
		switch {
		case errors.Is(err, storage.ErrNotFound):
			fail(w, http.StatusConflict, p.name+" hasn't been uploaded yet")
			return
		case err != nil:
			serverError(w, err)
			return
		case problem != "":
			fail(w, http.StatusUnprocessableEntity, p.name+" isn't what was declared: "+problem)
			return
		}
	}
	before, err := s.Store.FinishCapture(ctx, u.ID, id, req.Version, s.Now())
	if err != nil {
		captureFailed(w, err)
		return
	}
	if before.Version != 0 {
		s.removeVersion(ctx, u.ID, id, before.Version)
	}
	reply(w, http.StatusOK, map[string]any{"version": req.Version})
}

// changes lists what changed after `since`, with thumbnail URLs for
// what's still there, and where to carry on from.
func (s *Server) changes(w http.ResponseWriter, r *http.Request, u store.User) {
	q := r.URL.Query()
	since, err := strconv.ParseInt(q.Get("since"), 10, 64)
	if q.Get("since") == "" {
		since, err = 0, nil
	}
	limit, lerr := strconv.Atoi(q.Get("limit"))
	if q.Get("limit") == "" {
		limit, lerr = MaxChanges, nil
	}
	if err != nil || lerr != nil || since < 0 || limit < 1 || limit > MaxChanges {
		failWith(w, badRequest("since is a change number, limit 1 to 500"))
		return
	}
	ctx := r.Context()
	captures, err := s.Store.Changes(ctx, u.ID, since, limit+1)
	if err != nil {
		serverError(w, err)
		return
	}
	more := len(captures) > limit
	if more {
		captures = captures[:limit]
	}
	out := make([]captureJSON, 0, len(captures))
	next := since
	for _, c := range captures {
		j := captureJSON{
			ID: c.ID, Version: c.Version, Seq: c.Seq, Taken: c.Taken, Updated: c.Updated,
			Size: c.Sizes.Total(), Trashed: c.Trashed, Gone: c.Gone,
		}
		if !c.Gone {
			j.ThumbURL, err = s.Storage.PresignGet(ctx, captureKey(u.ID, c.ID, c.Version, "thumb"), GetTTL)
			if err != nil {
				serverError(w, err)
				return
			}
		}
		out = append(out, j)
		next = c.Seq
	}
	reply(w, http.StatusOK, map[string]any{"captures": out, "next": next, "more": more})
}

// capturePart is a download URL for a part of a capture's live version.
func (s *Server) capturePart(w http.ResponseWriter, r *http.Request, u store.User) {
	name := r.PathValue("part")
	known := false
	for _, p := range parts {
		known = known || p.name == name
	}
	ctx := r.Context()
	c, err := s.Store.Capture(ctx, u.ID, r.PathValue("id"))
	if err == nil && (c.Version == 0 || !known) {
		err = store.ErrNotFound
	} else if err == nil && c.Gone {
		err = store.ErrGone
	}
	if err != nil {
		captureFailed(w, err)
		return
	}
	url, err := s.Storage.PresignGet(ctx, captureKey(u.ID, c.ID, c.Version, name), GetTTL)
	if err != nil {
		serverError(w, err)
		return
	}
	reply(w, http.StatusOK, map[string]any{
		"url": url, "version": c.Version, "size": partSize(c.Sizes, name), "expires": s.Now().Add(GetTTL),
	})
}

// trashCapture puts a capture in the trash, or with ?forever=true deletes
// it for good at once.
func (s *Server) trashCapture(w http.ResponseWriter, r *http.Request, u store.User) {
	ctx, id := r.Context(), r.PathValue("id")
	var err error
	if r.URL.Query().Get("forever") == "true" {
		var was store.Capture
		if was, err = s.Store.PurgeCapture(ctx, u.ID, id); err == nil {
			s.removeCapture(ctx, was)
		}
	} else {
		err = s.Store.TrashCapture(ctx, u.ID, id, s.Now())
	}
	if err != nil {
		captureFailed(w, err)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

func (s *Server) restoreCapture(w http.ResponseWriter, r *http.Request, u store.User) {
	if err := s.Store.RestoreCapture(r.Context(), u.ID, r.PathValue("id")); err != nil {
		captureFailed(w, err)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

// removeVersion deletes a version's parts; a failure is only logged.
func (s *Server) removeVersion(ctx context.Context, owner int64, id string, version int64) {
	for _, p := range parts {
		key := captureKey(owner, id, version, p.name)
		if err := s.Storage.Delete(ctx, key); err != nil {
			log.Printf("removing %s: %v", key, err)
		}
	}
}

// removeCapture deletes everything a purged capture held.
func (s *Server) removeCapture(ctx context.Context, was store.Capture) {
	for _, v := range []int64{was.Version, was.Pending} {
		if v != 0 {
			s.removeVersion(ctx, was.Owner, was.ID, v)
		}
	}
}

// sweepCaptures purges captures a month in the trash and drops uploads
// never finished, a batch at a time. Returns how many of each.
func (s *Server) sweepCaptures(ctx context.Context) (purged, dropped int, err error) {
	now := s.Now()
	for {
		gone, abandoned, err := s.Store.SweepCaptures(ctx, now.AddDate(0, 0, -TrashDays), now.Add(-Grace), SweepBatch)
		if err != nil {
			return purged, dropped, err
		}
		for _, c := range gone {
			s.removeCapture(ctx, c)
		}
		for _, c := range abandoned {
			s.removeVersion(ctx, c.Owner, c.ID, c.Pending)
		}
		purged, dropped = purged+len(gone), dropped+len(abandoned)
		if len(gone) < SweepBatch && len(abandoned) < SweepBatch {
			return purged, dropped, nil
		}
	}
}
