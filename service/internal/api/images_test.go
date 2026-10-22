package api

import (
	"bytes"
	"context"
	"encoding/binary"
	"hash/crc32"
	"image"
	"image/color"
	"image/jpeg"
	stdpng "image/png"
	"strings"
	"testing"
)

func encodePNG(w, h int) []byte {
	img := image.NewNRGBA(image.Rect(0, 0, w, h))
	for i := range img.Pix {
		img.Pix[i] = byte(i)
	}
	var b bytes.Buffer
	_ = stdpng.Encode(&b, img)
	return b.Bytes()
}

func encodeJPEG(w, h int) []byte {
	img := image.NewRGBA(image.Rect(0, 0, w, h))
	for x := 0; x < w; x++ {
		img.Set(x, x%h, color.RGBA{200, 10, 10, 255})
	}
	var b bytes.Buffer
	_ = jpeg.Encode(&b, img, nil)
	return b.Bytes()
}

// chunk is a PNG chunk with a right CRC.
func chunk(typ string, body []byte) []byte {
	out := binary.BigEndian.AppendUint32(nil, uint32(len(body)))
	out = append(out, typ...)
	out = append(out, body...)
	return binary.BigEndian.AppendUint32(out, crc32.ChecksumIEEE(append([]byte(typ), body...)))
}

// withChunk puts `c` just before a PNG's IEND.
func withChunk(p, c []byte) []byte {
	end := len(p) - 12
	return append(append(append([]byte{}, p[:end]...), c...), p[end:]...)
}

func TestImagesAreCheckedThroughAndThrough(t *testing.T) {
	ctx := context.Background()
	good := encodePNG(64, 48)
	jpg := encodeJPEG(64, 48)
	zip := []byte("PK\x03\x04 a zip file, say")

	// IHDR claiming 10000x10000; the size check stops it before a decode.
	ihdr := binary.BigEndian.AppendUint32(binary.BigEndian.AppendUint32(nil, 10000), 10000)
	ihdr = append(ihdr, 8, 6, 0, 0, 0)
	huge := append([]byte("\x89PNG\r\n\x1a\n"), chunk("IHDR", ihdr)...)
	huge = append(huge, chunk("IDAT", nil)...)
	huge = append(huge, chunk("IEND", nil)...)

	badCRC := append([]byte{}, good...)
	badCRC[len(badCRC)-20] ^= 0xff

	for _, c := range []struct {
		name, typ string
		data      []byte
		want      string // "" for accepted
	}{
		{"a PNG", "image/png", good, ""},
		{"a JPEG", "image/jpeg", jpg, ""},
		{"a little metadata", "image/png", withChunk(good, chunk("tEXt", []byte("Software\x00Spooch"))), ""},
		{"a zip after a PNG", "image/png", append(append([]byte{}, good...), zip...), "after the image"},
		{"a zip after a JPEG", "image/jpeg", append(append([]byte{}, jpg...), zip...), "after the image"},
		{"a zip in a chunk", "image/png", withChunk(good, chunk("zIPp", bytes.Repeat(zip, 20000))), "metadata"},
		{"a cut-off PNG", "image/png", good[:len(good)-30], ""},
		{"a cut-off JPEG", "image/jpeg", jpg[:len(jpg)-30], ""},
		{"a bad CRC", "image/png", badCRC, "CRC"},
		{"a JPEG called a PNG", "image/png", jpg, "isn't a PNG"},
		{"too many pixels", "image/png", huge, "megapixels"},
		{"HTML", "image/png", []byte("<html><script>"), "isn't a PNG"},
	} {
		err := checkImage(ctx, c.typ, c.data)
		switch {
		case c.name == "a cut-off PNG" || c.name == "a cut-off JPEG":
			if err == nil {
				t.Errorf("%s: accepted", c.name)
			}
		case c.want == "" && err != nil:
			t.Errorf("%s: refused: %v", c.name, err)
		case c.want != "" && (err == nil || !strings.Contains(err.Error(), c.want)):
			t.Errorf("%s: %v, want %q", c.name, err, c.want)
		}
	}
}

func TestACaptureDocMustBeJSON(t *testing.T) {
	f := setup(t)
	f.files.Put("k", []byte("{not json"))
	if problem, err := f.srv.checkUpload(context.Background(), "k", 9, "application/json"); err != nil || problem == "" {
		t.Fatalf("took a broken doc: %q %v", problem, err)
	}
	f.files.Put("k", []byte(`{"a":1}`))
	if problem, err := f.srv.checkUpload(context.Background(), "k", 7, "application/json"); err != nil || problem != "" {
		t.Fatalf("refused a doc: %q %v", problem, err)
	}
}
