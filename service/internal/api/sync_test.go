package api

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/ycamper/spooch/service/internal/token"
)

// body decodes a reply.
func body(t *testing.T, w *httptest.ResponseRecorder) map[string]any {
	t.Helper()
	var out map[string]any
	if err := json.Unmarshal(w.Body.Bytes(), &out); err != nil {
		t.Fatalf("not JSON: %s", w.Body)
	}
	return out
}

func TestAccountsSignUpSignInAndSignOut(t *testing.T) {
	f := setup(t)
	up := map[string]any{"name": "dana", "password": "a good long one", "code": "letmein", "device": "laptop"}
	if w := f.do("POST", "/v1/accounts", "", up); w.Code != http.StatusForbidden {
		t.Fatalf("sign-ups open with no code set: %d", w.Code)
	}
	f.srv.SignupCode = "letmein"
	for field, bad := range map[string]any{"code": "nope", "name": "Dana!", "password": "short"} {
		req := map[string]any{}
		for k, v := range up {
			req[k] = v
		}
		req[field] = bad
		if w := f.do("POST", "/v1/accounts", "", req); w.Code < 400 || w.Code > 403 {
			t.Fatalf("a bad %s: %d %s", field, w.Code, w.Body)
		}
	}
	w := f.do("POST", "/v1/accounts", "", up)
	if w.Code != http.StatusCreated {
		t.Fatalf("sign up: %d %s", w.Code, w.Body)
	}
	laptop := body(t, w)["token"].(string)
	if w := f.do("GET", "/v1/me", laptop, nil); w.Code != http.StatusOK || body(t, w)["name"] != "dana" {
		t.Fatalf("the new token: %d %s", w.Code, w.Body)
	}
	if w := f.do("POST", "/v1/accounts", "", up); w.Code != http.StatusConflict {
		t.Fatalf("a second dana: %d", w.Code)
	}

	// Signing in: a wrong password, a wrong name, then right.
	in := map[string]any{"name": "dana", "password": "a good long one", "device": "desk"}
	if w := f.do("POST", "/v1/sign-in", "", map[string]any{"name": "dana", "password": "wrong one!", "device": "desk"}); w.Code != http.StatusUnauthorized {
		t.Fatalf("a wrong password: %d", w.Code)
	}
	if w := f.do("POST", "/v1/sign-in", "", map[string]any{"name": "nobody", "password": "a good long one", "device": "desk"}); w.Code != http.StatusUnauthorized {
		t.Fatalf("a wrong name: %d", w.Code)
	}
	w = f.do("POST", "/v1/sign-in", "", in)
	if w.Code != http.StatusOK {
		t.Fatalf("sign in: %d %s", w.Code, w.Body)
	}
	desk := body(t, w)["token"].(string)

	// Devices: two; the laptop signs the desk out.
	devices := body(t, f.do("GET", "/v1/devices", laptop, nil))["devices"].([]any)
	if len(devices) != 2 {
		t.Fatalf("devices: %v", devices)
	}
	deskID := body(t, w)["device"].(float64)
	if w := f.do("DELETE", "/v1/devices/"+jsonNumber(deskID), laptop, nil); w.Code != http.StatusNoContent {
		t.Fatalf("sign the desk out: %d", w.Code)
	}
	if f.do("GET", "/v1/me", desk, nil).Code != http.StatusUnauthorized {
		t.Fatal("a signed-out device still works")
	}
	if w := f.do("DELETE", "/v1/devices/"+jsonNumber(deskID), f.device, nil); w.Code != http.StatusNotFound {
		t.Fatalf("another account signs a device out: %d", w.Code)
	}
}

func jsonNumber(n float64) string {
	b, _ := json.Marshal(n)
	return string(b)
}

func TestTooManyWrongPasswordsLockAnAccountForAWhile(t *testing.T) {
	f := setup(t)
	// mark was made by hand: he sets a password with his token.
	if w := f.do("PUT", "/v1/password", f.device, map[string]any{"password": "the right one"}); w.Code != http.StatusNoContent {
		t.Fatalf("set a password: %d %s", w.Code, w.Body)
	}
	if w := f.do("PUT", "/v1/password", f.device, map[string]any{"current": "nope", "password": "another one!"}); w.Code != http.StatusForbidden {
		t.Fatalf("change it without the current one: %d", w.Code)
	}
	try := func(p string) int {
		return f.do("POST", "/v1/sign-in", "", map[string]any{"name": "mark", "password": p, "device": "x"}).Code
	}
	for range MaxFailures {
		if c := try("wrong password"); c != http.StatusUnauthorized {
			t.Fatalf("a wrong password: %d", c)
		}
	}
	if c := try("the right one"); c != http.StatusTooManyRequests {
		t.Fatalf("locked out, the right password: %d", c)
	}
	f.now = f.now.Add(Lockout)
	if c := try("the right one"); c != http.StatusOK {
		t.Fatalf("after the lockout: %d", c)
	}
}

// keyOf is the storage key a memory upload or download URL names.
func keyOf(t *testing.T, u string) string {
	t.Helper()
	i := strings.LastIndex(u, "/")
	k, err := url.PathUnescape(u[i+1:])
	if err != nil {
		t.Fatal(err)
	}
	return k
}

var doc = []byte(`{"version":1}`)

// upload begins a version of capture `id` on `base` and uploads its
// parts; returns the new version and the reply.
func (f *fixture) upload(id string, base int64, source []byte) (int64, map[string]any) {
	f.t.Helper()
	w := f.do("PUT", "/v1/captures/"+id, f.device, map[string]any{
		"base": base, "taken": f.now, "doc_size": len(doc), "source_size": len(source), "thumb_size": len(png),
	})
	if w.Code != http.StatusOK {
		return 0, map[string]any{"code": w.Code, "body": w.Body.String()}
	}
	b := body(f.t, w)
	uploads := b["uploads"].(map[string]any)
	for part, data := range map[string][]byte{"doc": doc, "source": source, "thumb": png} {
		f.files.Put(keyOf(f.t, uploads[part].(string)), data)
	}
	return int64(b["version"].(float64)), b
}

func (f *fixture) finish(id string, version int64) int {
	return f.do("POST", "/v1/captures/"+id+"/finish", f.device, map[string]any{"version": version}).Code
}

func (f *fixture) changes(since int64) []any {
	f.t.Helper()
	w := f.do("GET", "/v1/captures?since="+jsonNumber(float64(since)), f.device, nil)
	if w.Code != http.StatusOK {
		f.t.Fatalf("changes: %d %s", w.Code, w.Body)
	}
	return body(f.t, w)["captures"].([]any)
}

func TestACaptureSyncsVersionsTrashAndAll(t *testing.T) {
	f := setup(t)
	f.srv.Quota = 1 << 20
	id := token.Random(CaptureIDLen)
	if w := f.do("PUT", "/v1/captures/short", f.device, map[string]any{}); w.Code != http.StatusBadRequest {
		t.Fatalf("a bad id: %d", w.Code)
	}

	// The first version: nothing until finished.
	v1, b := f.upload(id, 0, png)
	if v1 == 0 {
		t.Fatalf("begin: %v", b)
	}
	if len(f.changes(0)) != 0 {
		t.Fatal("an unfinished capture is listed")
	}
	if c := f.finish(id, v1); c != http.StatusOK {
		t.Fatalf("finish: %d", c)
	}
	ch := f.changes(0)
	if len(ch) != 1 {
		t.Fatalf("changes: %v", ch)
	}
	c := ch[0].(map[string]any)
	if c["id"] != id || c["thumb_url"] == "" || c["size"].(float64) != float64(len(doc)+2*len(png)) {
		t.Fatalf("listed: %v", c)
	}
	seq := int64(c["seq"].(float64))

	// A device that hasn't seen v1 conflicts; one that has makes v2, and
	// v1's parts go.
	if _, b := f.upload(id, 0, png); b["code"] != http.StatusConflict {
		t.Fatalf("a stale base: %v", b)
	}
	v2, _ := f.upload(id, v1, png)
	if c := f.finish(id, v1); c != http.StatusConflict {
		t.Fatalf("finishing the old version: %d", c)
	}
	if c := f.finish(id, v2); c != http.StatusOK {
		t.Fatalf("finish v2: %d", c)
	}
	if f.files.Has(captureKey(1, id, v1, "source")) {
		t.Fatal("v1's image is left behind")
	}
	if ch := f.changes(seq); len(ch) != 1 || ch[0].(map[string]any)["version"].(float64) != float64(v2) {
		t.Fatalf("changes since v1: %v", ch)
	}

	// Its full image downloads on request.
	w := f.do("GET", "/v1/captures/"+id+"/source", f.device, nil)
	if w.Code != http.StatusOK || keyOf(t, body(t, w)["url"].(string)) != captureKey(1, id, v2, "source") {
		t.Fatalf("the source: %d %s", w.Code, w.Body)
	}

	// An upload that isn't what it said is refused; a missing one too.
	other := token.Random(CaptureIDLen)
	v, _ := f.upload(other, 0, bytes.Repeat([]byte{1}, len(png)))
	if c := f.finish(other, v); c != http.StatusUnprocessableEntity {
		t.Fatalf("not a PNG: %d", c)
	}
	missing := token.Random(CaptureIDLen)
	vm, _ := f.upload(missing, 0, png)
	f.files.Delete(context.Background(), captureKey(1, missing, vm, "thumb"))
	if c := f.finish(missing, vm); c != http.StatusConflict {
		t.Fatalf("not uploaded: %d", c)
	}

	// The trash: listed as trashed, unchangeable, restorable.
	if w := f.do("DELETE", "/v1/captures/"+id, f.device, nil); w.Code != http.StatusNoContent {
		t.Fatalf("trash: %d", w.Code)
	}
	if ch := f.changes(seq); ch[len(ch)-1].(map[string]any)["trashed"] == nil {
		t.Fatalf("not listed as trashed: %v", ch)
	}
	if _, b := f.upload(id, v2, png); b["code"] != http.StatusConflict {
		t.Fatalf("changed in the trash: %v", b)
	}
	if w := f.do("POST", "/v1/captures/"+id+"/restore", f.device, nil); w.Code != http.StatusNoContent {
		t.Fatalf("restore: %d", w.Code)
	}

	// The account fills up.
	if _, b := f.upload(token.Random(CaptureIDLen), 0, bytes.Repeat(png, 20000)); b["code"] != http.StatusInsufficientStorage {
		t.Fatalf("over the quota: %v", b)
	}

	// Deleted forever: its parts go, and the changes say so.
	if w := f.do("DELETE", "/v1/captures/"+id+"?forever=true", f.device, nil); w.Code != http.StatusNoContent {
		t.Fatalf("delete forever: %d", w.Code)
	}
	if f.files.Has(captureKey(1, id, v2, "source")) {
		t.Fatal("deleted forever, but its image stays")
	}
	if w := f.do("GET", "/v1/captures/"+id+"/source", f.device, nil); w.Code != http.StatusGone {
		t.Fatalf("a deleted capture's image: %d", w.Code)
	}
	last := f.changes(seq)
	if gone := last[len(last)-1].(map[string]any); gone["gone"] != true || gone["thumb_url"] != nil {
		t.Fatalf("not listed as gone: %v", gone)
	}

	// Another account sees none of it.
	if w := f.do("GET", "/v1/captures", f.another("zed"), nil); len(body(t, w)["captures"].([]any)) != 0 {
		t.Fatal("another account sees the captures")
	}
}

func TestTheSweeperEmptiesTheTrashAfterAMonth(t *testing.T) {
	f := setup(t)
	f.srv.Quota = 1 << 30
	id := token.Random(CaptureIDLen)
	v, _ := f.upload(id, 0, png)
	f.finish(id, v)
	f.do("DELETE", "/v1/captures/"+id, f.device, nil)
	abandoned := token.Random(CaptureIDLen)
	va, _ := f.upload(abandoned, 0, png)

	f.now = f.now.Add(2 * time.Hour)
	if _, n, err := f.srv.Sweep(context.Background()); err != nil || n != 1 {
		t.Fatalf("swept %d captures (%v), want the abandoned upload", n, err)
	}
	if f.files.Has(captureKey(1, abandoned, va, "source")) || !f.files.Has(captureKey(1, id, v, "source")) {
		t.Fatal("the sweeper took the wrong one")
	}
	f.now = f.now.AddDate(0, 0, TrashDays)
	if _, n, err := f.srv.Sweep(context.Background()); err != nil || n != 1 {
		t.Fatalf("swept %d captures (%v), want the trashed one", n, err)
	}
	if f.files.Has(captureKey(1, id, v, "source")) {
		t.Fatal("a month in the trash, its image stays")
	}
}
