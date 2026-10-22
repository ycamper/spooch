package api

import (
	"errors"
	"html/template"
	"io"
	"net/http"
	"strconv"
	"strings"

	"github.com/ycamper/spooch/service/internal/store"
	"github.com/ycamper/spooch/service/internal/token"
)

// pagePolicy lets a page show its image and inline style, and nothing else.
const pagePolicy = "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; " +
	"frame-ancestors 'none'; base-uri 'none'; form-action 'none'"

// linkCache lets Cloudflare keep a public image briefly, so stopping a
// share takes effect within a minute.
const linkCache = "public, max-age=60"

// view serves `/<id><secret>`, a page showing the image with a preview
// for chat apps, and `/<id><secret>.png` (or `.jpg`), the image itself.
func (s *Server) view(w http.ResponseWriter, r *http.Request) {
	base, ext, image := strings.Cut(r.PathValue("link"), ".")
	id, secret, ok := token.SplitLink(base)
	if !ok {
		http.NotFound(w, r)
		return
	}
	share, err := s.Store.Share(r.Context(), id)
	if err != nil && !errors.Is(err, store.ErrNotFound) {
		serverError(w, err)
		return
	}
	if err != nil || share.State != store.Live || s.expired(share) ||
		!token.Same(share.SecretHash, token.Hash(secret)) {
		http.NotFound(w, r)
		return
	}
	switch {
	case !share.Public:
		// Shared with accounts only: they see it in Spooch until there's
		// a sign-in on the web.
		html(w, http.StatusForbidden)
		_, _ = io.WriteString(w, restricted)
	case !image:
		html(w, http.StatusOK)
		_ = pageTemplate.Execute(w, map[string]string{
			"Image": s.PublicURL + "/" + base + "." + types[share.Type],
		})
	case ext != types[share.Type]:
		http.NotFound(w, r)
	default:
		s.serveImage(w, r, share, linkCache)
	}
}

func html(w http.ResponseWriter, status int) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Header().Set("Content-Security-Policy", pagePolicy)
	w.WriteHeader(status)
}

// serveImage writes a share's image, cached as `cache` says.
func (s *Server) serveImage(w http.ResponseWriter, r *http.Request, share store.Share, cache string) {
	f, err := s.Storage.Open(r.Context(), shareKey(share.ID))
	if err != nil {
		serverError(w, err)
		return
	}
	defer f.Close()
	h := w.Header()
	h.Set("Content-Type", share.Type)
	h.Set("X-Content-Type-Options", "nosniff")
	h.Set("Content-Length", strconv.FormatInt(share.Size, 10))
	h.Set("Content-Disposition", `inline; filename="spooch.`+types[share.Type]+`"`)
	h.Set("Cache-Control", cache)
	_, _ = io.Copy(w, f)
}

func (s *Server) home(w http.ResponseWriter, _ *http.Request) {
	w.Header().Set("Content-Type", "text/plain; charset=utf-8")
	_, _ = io.WriteString(w, "Spooch\n")
}

const restricted = `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="robots" content="noindex, nofollow">
<meta name="viewport" content="width=device-width, initial-scale=1"><title>Spooch</title>
<style>body{margin:0;min-height:100vh;display:grid;place-items:center;background:#121317;color:#e9eaee;font:16px/1.5 system-ui,sans-serif}p{max-width:28em;padding:0 16px}</style>
</head><body><p>This capture is shared with specific Spooch accounts. Open Spooch to see it in your gallery under Shared with me.</p></body></html>
`

var pageTemplate = template.Must(template.New("page").Parse(`<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex, nofollow">
<title>Spooch</title>
<meta property="og:type" content="website">
<meta property="og:title" content="Spooch capture">
<meta property="og:image" content="{{.Image}}">
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:image" content="{{.Image}}">
<style>
  html, body { margin: 0; height: 100%; background: #121317; }
  body { display: grid; place-items: center; }
  img { max-width: 100vw; max-height: 100vh; display: block; }
</style>
</head>
<body><img src="{{.Image}}" alt="A Spooch capture"></body>
</html>
`))
