package api

import (
	"encoding/json"
	"errors"
	"io"
	"log"
	"net/http"

	"github.com/ycamper/spooch/service/internal/store"
)

// badRequest is a problem with what the app sent, told back as it is.
type badRequest string

func (e badRequest) Error() string { return string(e) }

func reply(w http.ResponseWriter, status int, body any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	// Upload URLs carry query strings; keep their & as is.
	enc := json.NewEncoder(w)
	enc.SetEscapeHTML(false)
	_ = enc.Encode(body)
}

func fail(w http.ResponseWriter, status int, msg string) {
	reply(w, status, map[string]string{"error": msg})
}

// serverError logs what went wrong and tells the app no more than that.
func serverError(w http.ResponseWriter, err error) {
	log.Printf("error: %v", err)
	fail(w, http.StatusInternalServerError, "something went wrong on our side")
}

// failWith replies to `err`: a bad request as it is, a missing share as
// 404, anything else as our fault.
func failWith(w http.ResponseWriter, err error) {
	var bad badRequest
	switch {
	case errors.As(err, &bad):
		fail(w, http.StatusBadRequest, bad.Error())
	case errors.Is(err, store.ErrNotFound):
		fail(w, http.StatusNotFound, "no such share")
	default:
		serverError(w, err)
	}
}

// decode reads a JSON body of at most `limit` bytes into `v`, with no
// fields `v` doesn't have. It replies and returns false if it can't.
func decode(w http.ResponseWriter, r *http.Request, limit int64, v any) bool {
	dec := json.NewDecoder(http.MaxBytesReader(w, r.Body, limit))
	dec.DisallowUnknownFields()
	err := dec.Decode(v)
	if err == nil && dec.Decode(&struct{}{}) != io.EOF {
		err = errors.New("more than one JSON value")
	}
	var tooBig *http.MaxBytesError
	switch {
	case errors.As(err, &tooBig):
		fail(w, http.StatusRequestEntityTooLarge, "the request is too large")
	case err != nil:
		fail(w, http.StatusBadRequest, "the request isn't the JSON expected")
	default:
		return true
	}
	return false
}
