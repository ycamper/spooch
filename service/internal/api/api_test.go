package api

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/ycamper/spooch/service/internal/storage"
	"github.com/ycamper/spooch/service/internal/store"
	"github.com/ycamper/spooch/service/internal/token"
)

var png = append([]byte{0x89, 'P', 'N', 'G', '\r', '\n', 0x1a, '\n'}, bytes.Repeat([]byte{7}, 100)...)

type fixture struct {
	t      *testing.T
	srv    *Server
	db     *store.Memory
	files  *storage.Memory
	http   http.Handler
	device string
	now    time.Time
}

func setup(t *testing.T) *fixture {
	f := &fixture{t: t, db: store.NewMemory(), files: storage.NewMemory(), now: time.Date(2026, 10, 5, 12, 0, 0, 0, time.UTC)}
	f.srv = &Server{Store: f.db, Storage: f.files, PublicURL: "https://spooch.test", Now: func() time.Time { return f.now }}
	f.http = f.srv.Handler()
	u, _ := f.db.AddUser(context.Background(), "mark")
	f.device = token.NewDevice()
	_, _ = f.db.AddDevice(context.Background(), u.ID, "laptop", token.Hash(f.device))
	return f
}

func (f *fixture) do(method, path, device string, body any) *httptest.ResponseRecorder {
	var r io.Reader
	if body != nil {
		b, _ := json.Marshal(body)
		r = bytes.NewReader(b)
	}
	req := httptest.NewRequest(method, path, r)
	if device != "" {
		req.Header.Set("Authorization", "Bearer "+device)
	}
	w := httptest.NewRecorder()
	f.http.ServeHTTP(w, req)
	return w
}

// share runs the whole flow: create, upload `data`, finalize. Returns the
// link's path.
func (f *fixture) share(data []byte, expiryDays int) (string, *httptest.ResponseRecorder) {
	id, secret := token.Random(token.IDLen), token.Random(token.SecretLen)
	w := f.do("POST", "/v1/shares", f.device, map[string]any{
		"id": id, "secret_hash": token.Hash(secret), "size": len(png), "type": "image/png", "expiry_days": expiryDays,
	})
	if w.Code != http.StatusCreated {
		f.t.Fatalf("create: %d %s", w.Code, w.Body)
	}
	if strings.Contains(w.Body.String(), `&`) {
		f.t.Fatalf("the reply escapes &: %s", w.Body)
	}
	f.files.Put(uploadKey(id), data)
	return "/" + id + secret, f.do("POST", "/v1/shares/"+id+"/finalize", f.device, nil)
}

func TestAShareGoesLiveAndItsLinkWorks(t *testing.T) {
	f := setup(t)
	link, w := f.share(png, 0)
	if w.Code != http.StatusOK {
		t.Fatalf("finalize: %d %s", w.Code, w.Body)
	}
	page := f.do("GET", link, "", nil)
	if page.Code != http.StatusOK || !strings.Contains(page.Body.String(), `og:image" content="https://spooch.test`+link+`.png"`) {
		t.Fatalf("page: %d %s", page.Code, page.Body)
	}
	if page.Header().Get("X-Robots-Tag") == "" || page.Header().Get("Referrer-Policy") != "no-referrer" {
		t.Fatal("the page isn't kept private")
	}
	img := f.do("GET", link+".png", "", nil)
	if img.Code != http.StatusOK || !bytes.Equal(img.Body.Bytes(), png) || img.Header().Get("Content-Type") != "image/png" {
		t.Fatalf("image: %d %q", img.Code, img.Header())
	}
	// The checked upload became the share's image; the upload is gone.
	if f.files.Has(uploadKey(link[1:11])) || !f.files.Has(shareKey(link[1:11])) {
		t.Fatal("the upload wasn't moved into place")
	}
	// A wrong secret, a wrong extension, or nonsense: not found.
	wrong := link[:len(link)-1] + "x"
	if wrong == link {
		wrong = link[:len(link)-1] + "y"
	}
	for _, bad := range []string{wrong, link + ".jpg", "/nope"} {
		if got := f.do("GET", bad, "", nil).Code; got != http.StatusNotFound {
			t.Fatalf("%s gave %d", bad, got)
		}
	}
	list := f.do("GET", "/v1/shares", f.device, nil)
	if !strings.Contains(list.Body.String(), link[1:11]) {
		t.Fatalf("the share isn't listed: %s", list.Body)
	}
}

func TestDeviceTokensAreChecked(t *testing.T) {
	f := setup(t)
	if f.do("GET", "/v1/me", "", nil).Code != http.StatusUnauthorized {
		t.Fatal("no token was let in")
	}
	if f.do("GET", "/v1/me", token.NewDevice(), nil).Code != http.StatusUnauthorized {
		t.Fatal("an unknown token was let in")
	}
	me := f.do("GET", "/v1/me", f.device, nil)
	if me.Code != http.StatusOK || !strings.Contains(me.Body.String(), `{"links":"https://spooch.test","name":"mark"}`) {
		t.Fatalf("me: %d %s", me.Code, me.Body)
	}
	_ = f.db.RevokeDevice(context.Background(), 1)
	if f.do("GET", "/v1/me", f.device, nil).Code != http.StatusUnauthorized {
		t.Fatal("a revoked token was let in")
	}
}

func TestAnUploadThatIsntTheDeclaredImageIsRefused(t *testing.T) {
	f := setup(t)
	link, w := f.share([]byte("<html>not a png, but the same length......................................................................"), 0)
	if w.Code != http.StatusUnprocessableEntity {
		t.Fatalf("finalize took a fake: %d %s", w.Code, w.Body)
	}
	if f.files.Has(uploadKey(link[1:11])) || f.do("GET", link, "", nil).Code != http.StatusNotFound {
		t.Fatal("the fake was kept")
	}
	_, w = f.share(png[:50], 0)
	if w.Code != http.StatusUnprocessableEntity {
		t.Fatalf("finalize took the wrong size: %d", w.Code)
	}
}

func TestRequestsAreValidated(t *testing.T) {
	f := setup(t)
	ok := map[string]any{"id": token.Random(10), "secret_hash": token.Hash("s"), "size": 10, "type": "image/png"}
	for field, bad := range map[string]any{"id": "short", "secret_hash": "nothex", "size": MaxSize + 1, "type": "text/html", "expiry_days": -2} {
		body := map[string]any{}
		for k, v := range ok {
			body[k] = v
		}
		body[field] = bad
		if w := f.do("POST", "/v1/shares", f.device, body); w.Code < 400 {
			t.Fatalf("bad %s accepted: %d", field, w.Code)
		}
	}
	if w := f.do("POST", "/v1/shares", f.device, ok); w.Code != http.StatusCreated {
		t.Fatalf("good request refused: %d %s", w.Code, w.Body)
	}
	if w := f.do("POST", "/v1/shares", f.device, ok); w.Code != http.StatusConflict {
		t.Fatalf("a taken id was accepted: %d", w.Code)
	}
}

func TestDeleteExpiryAndTheSweeper(t *testing.T) {
	f := setup(t)
	// Delete from the app.
	link, _ := f.share(png, 0)
	id := link[1:11]
	if w := f.do("DELETE", "/v1/shares/"+id, f.device, nil); w.Code != http.StatusNoContent {
		t.Fatalf("delete: %d", w.Code)
	}
	if f.files.Has(shareKey(id)) || f.do("GET", link, "", nil).Code != http.StatusNotFound {
		t.Fatal("a deleted share still works")
	}
	// A one-day link stops working after a day, and the sweeper clears it;
	// so does a share that was never finalized.
	short, _ := f.share(png, 1)
	never, _ := f.share(png, -1)
	other := token.Random(10)
	f.do("POST", "/v1/shares", f.device, map[string]any{"id": other, "secret_hash": token.Hash("s"), "size": 5, "type": "image/png"})
	f.now = f.now.Add(48 * time.Hour)
	if f.do("GET", short, "", nil).Code != http.StatusNotFound {
		t.Fatal("an expired link still works")
	}
	n, _, err := f.srv.Sweep(context.Background())
	if err != nil || n != 2 {
		t.Fatalf("swept %d (%v), want 2", n, err)
	}
	if f.files.Has(shareKey(short[1:11])) || f.do("GET", never, "", nil).Code != http.StatusOK {
		t.Fatal("the sweeper got the wrong shares")
	}
}

// another adds an account and returns its device token.
func (f *fixture) another(name string) string {
	u, _ := f.db.AddUser(context.Background(), name)
	t := token.NewDevice()
	_, _ = f.db.AddDevice(context.Background(), u.ID, "phone", token.Hash(t))
	return t
}

// shareWith creates and finalizes a share with `body` merged into the
// request. Returns its link path.
func (f *fixture) shareWith(body map[string]any) string {
	id, secret := token.Random(token.IDLen), token.Random(token.SecretLen)
	req := map[string]any{"id": id, "secret_hash": token.Hash(secret), "size": len(png), "type": "image/png"}
	for k, v := range body {
		req[k] = v
	}
	if w := f.do("POST", "/v1/shares", f.device, req); w.Code != http.StatusCreated {
		f.t.Fatalf("create: %d %s", w.Code, w.Body)
	}
	f.files.Put(uploadKey(id), png)
	if w := f.do("POST", "/v1/shares/"+id+"/finalize", f.device, nil); w.Code != http.StatusOK {
		f.t.Fatalf("finalize: %d %s", w.Code, w.Body)
	}
	return "/" + id + secret
}

func TestSharingWithAccountsOnly(t *testing.T) {
	f := setup(t)
	dana, sam := f.another("dana"), f.another("sam")
	if w := f.do("POST", "/v1/shares", f.device, map[string]any{
		"id": token.Random(10), "secret_hash": token.Hash("s"), "size": 5, "type": "image/png", "recipients": []string{"nobody"},
	}); w.Code != http.StatusBadRequest || !strings.Contains(w.Body.String(), "nobody") {
		t.Fatalf("an unknown recipient was accepted: %d %s", w.Code, w.Body)
	}
	link := f.shareWith(map[string]any{"public": false, "recipients": []string{"dana", "mark"}})
	id := link[1:11]
	// The link alone doesn't show it.
	if w := f.do("GET", link, "", nil); w.Code != http.StatusForbidden || !strings.Contains(w.Body.String(), "Shared with me") {
		t.Fatalf("the link showed an account-only share: %d", w.Code)
	}
	if f.do("GET", link+".png", "", nil).Code != http.StatusForbidden {
		t.Fatal("the image link showed an account-only share")
	}
	// Dana sees it in Spooch; Sam doesn't.
	got := f.do("GET", "/v1/shared-with-me", dana, nil)
	if !strings.Contains(got.Body.String(), id) || !strings.Contains(got.Body.String(), `"from":"mark"`) {
		t.Fatalf("dana's list: %s", got.Body)
	}
	if w := f.do("GET", "/v1/shares/"+id+"/image", dana, nil); w.Code != http.StatusOK || !bytes.Equal(w.Body.Bytes(), png) {
		t.Fatalf("dana's image: %d", w.Code)
	}
	if strings.Contains(f.do("GET", "/v1/shared-with-me", sam, nil).Body.String(), id) {
		t.Fatal("sam was shown a share that isn't his")
	}
	if f.do("GET", "/v1/shares/"+id+"/image", sam, nil).Code != http.StatusNotFound {
		t.Fatal("sam got the image")
	}
	// The owner sees who it's with (not themselves).
	if list := f.do("GET", "/v1/shares", f.device, nil).Body.String(); !strings.Contains(list, `"recipients":["dana"]`) || !strings.Contains(list, `"public":false`) {
		t.Fatalf("owner's list: %s", list)
	}
}

func TestChangingAndStoppingSharing(t *testing.T) {
	f := setup(t)
	dana := f.another("dana")
	link := f.shareWith(map[string]any{"expiry_days": 7})
	id := link[1:11]
	// Only the owner can change it.
	if f.do("PATCH", "/v1/shares/"+id, dana, map[string]any{"public": false}).Code != http.StatusNotFound {
		t.Fatal("someone else changed the share")
	}
	// Stop the link, share with dana, and make it last 2 days.
	w := f.do("PATCH", "/v1/shares/"+id, f.device, map[string]any{"public": false, "recipients": []string{"dana"}, "expiry_days": 2})
	if w.Code != http.StatusOK || !strings.Contains(w.Body.String(), `"recipients":["dana"]`) {
		t.Fatalf("change: %d %s", w.Code, w.Body)
	}
	if f.do("GET", link, "", nil).Code != http.StatusForbidden {
		t.Fatal("the link still works after it was stopped")
	}
	if f.do("GET", "/v1/shares/"+id+"/image", dana, nil).Code != http.StatusOK {
		t.Fatal("dana can't see what was shared with her")
	}
	// Unsharing from dana, and expiry, both stop her seeing it.
	f.do("PATCH", "/v1/shares/"+id, f.device, map[string]any{"recipients": []string{}})
	if f.do("GET", "/v1/shares/"+id+"/image", dana, nil).Code != http.StatusNotFound {
		t.Fatal("dana still sees it after it was unshared")
	}
	f.do("PATCH", "/v1/shares/"+id, f.device, map[string]any{"public": true})
	if f.do("GET", link, "", nil).Code != http.StatusOK {
		t.Fatal("the link didn't come back")
	}
	if f.do("PATCH", "/v1/shares/"+id, f.device, map[string]any{"expiry_days": 99999}).Code != http.StatusBadRequest {
		t.Fatal("a silly expiry was accepted")
	}
	f.now = f.now.Add(3 * 24 * time.Hour)
	if f.do("GET", link, "", nil).Code != http.StatusNotFound {
		t.Fatal("the link outlived its 2 days")
	}
	if f.do("GET", "/v1/shares/"+id+"/image", f.device, nil).Code != http.StatusNotFound {
		t.Fatal("the owner still gets an expired image")
	}
	// Expired, it can't be brought back.
	if f.do("PATCH", "/v1/shares/"+id, f.device, map[string]any{"expiry_days": 5}).Code != http.StatusGone {
		t.Fatal("an expired share was revived")
	}
}

func TestOnlyTheOwnerFinalizesOrDeletes(t *testing.T) {
	f := setup(t)
	dana := f.another("dana")
	link := f.shareWith(map[string]any{"recipients": []string{"dana"}})
	id := link[1:11]
	for _, method := range []string{"DELETE", "PATCH"} {
		if w := f.do(method, "/v1/shares/"+id, dana, map[string]any{}); w.Code != http.StatusNotFound {
			t.Fatalf("a recipient could %s: %d", method, w.Code)
		}
	}
	if f.do("POST", "/v1/shares/"+id+"/finalize", dana, nil).Code != http.StatusNotFound {
		t.Fatal("someone else finalized it")
	}
	if f.do("GET", link, "", nil).Code != http.StatusOK {
		t.Fatal("the share was harmed")
	}
}

func TestAStorageOutageDoesntLoseTheShare(t *testing.T) {
	f := setup(t)
	id, secret := token.Random(token.IDLen), token.Random(token.SecretLen)
	f.do("POST", "/v1/shares", f.device, map[string]any{"id": id, "secret_hash": token.Hash(secret), "size": len(png), "type": "image/png"})
	f.files.Put(uploadKey(id), png)
	f.files.Fail = errors.New("bucket unreachable")
	if w := f.do("POST", "/v1/shares/"+id+"/finalize", f.device, nil); w.Code != http.StatusInternalServerError ||
		strings.Contains(w.Body.String(), "unreachable") {
		t.Fatalf("finalize in an outage: %d %s", w.Code, w.Body)
	}
	f.files.Fail = nil
	if w := f.do("POST", "/v1/shares/"+id+"/finalize", f.device, nil); w.Code != http.StatusOK {
		t.Fatalf("finalize after the outage: %d %s", w.Code, w.Body)
	}
}

func TestALateUploadCantReplaceALiveImage(t *testing.T) {
	f := setup(t)
	link, _ := f.share(png, 0)
	f.files.Put(uploadKey(link[1:11]), []byte("something else"))
	if img := f.do("GET", link+".png", "", nil); !bytes.Equal(img.Body.Bytes(), png) {
		t.Fatal("the live image changed")
	}
}

func TestBodiesAreStrict(t *testing.T) {
	f := setup(t)
	good := map[string]any{"id": token.Random(10), "secret_hash": token.Hash("s"), "size": 10, "type": "image/png"}
	upper := map[string]any{"id": token.Random(10), "secret_hash": strings.ToUpper(token.Hash("s")), "size": 10, "type": "image/png"}
	extra := map[string]any{"id": token.Random(10), "secret_hash": token.Hash("s"), "size": 10, "type": "image/png", "admin": true}
	for name, body := range map[string]any{"uppercase hash": upper, "unknown field": extra} {
		if w := f.do("POST", "/v1/shares", f.device, body); w.Code != http.StatusBadRequest {
			t.Fatalf("%s accepted: %d", name, w.Code)
		}
	}
	good["recipients"] = []string{strings.Repeat("x", 8000)}
	if w := f.do("POST", "/v1/shares", f.device, good); w.Code != http.StatusRequestEntityTooLarge {
		t.Fatalf("a huge body: %d", w.Code)
	}
	if w := f.do("GET", "/v1/me", f.device, nil); w.Header().Get("X-Content-Type-Options") != "nosniff" {
		t.Fatal("the API doesn't say nosniff")
	}
}
