package api

import (
	"bytes"
	"context"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"hash/crc32"
	"image"
	"image/jpeg"
	stdpng "image/png"
	"io"
)

const (
	// MaxPixels is the largest image accepted. A full decode holds the
	// image in memory at about 4 bytes a pixel; three 4K screens are 25.
	MaxPixels = 50_000_000
	// MaxMetadata is how many bytes of metadata (PNG ancillary chunks,
	// JPEG APPn and COM segments) an image may carry, so none can carry a
	// file of its own.
	MaxMetadata = 256 << 10
)

// decodeSlots keeps full decodes to one at a time: each can take a couple
// of hundred MB.
var decodeSlots = make(chan struct{}, 1)

// checkUpload reads the object at `key` and says what's wrong with it as
// a `typ` of `size` bytes, or "" if nothing is. err is for the storage
// failing (ErrNotFound when nothing was uploaded).
func (s *Server) checkUpload(ctx context.Context, key string, size int64, typ string) (string, error) {
	n, err := s.Storage.Size(ctx, key)
	if err != nil {
		return "", err
	}
	if n != size {
		return fmt.Sprintf("it's %d bytes, not the %d declared", n, size), nil
	}
	f, err := s.Storage.Open(ctx, key)
	if err != nil {
		return "", err
	}
	defer f.Close()
	data, err := io.ReadAll(io.LimitReader(f, size+1))
	if err != nil {
		return "", err
	}
	if int64(len(data)) != size {
		return fmt.Sprintf("it's %d bytes, not the %d declared", len(data), size), nil
	}
	if typ == "application/json" {
		if !json.Valid(data) {
			return "it isn't JSON", nil
		}
		return "", nil
	}
	if err := checkImage(ctx, typ, data); err != nil {
		return err.Error(), nil
	}
	return "", nil
}

// checkImage reports what keeps `data` from being a plain image of type
// `typ`: its structure, nothing after its end, little metadata, a size
// within MaxPixels, and a full decode.
func checkImage(ctx context.Context, typ string, data []byte) error {
	var meta int
	var err error
	switch typ {
	case "image/png":
		meta, err = pngLayout(data)
	case "image/jpeg":
		meta, err = jpegLayout(data)
	default:
		return fmt.Errorf("%s isn't an image type we take", typ)
	}
	if err != nil {
		return err
	}
	if meta > MaxMetadata {
		return fmt.Errorf("it carries %d KB of metadata", meta>>10)
	}
	cfg, format, err := image.DecodeConfig(bytes.NewReader(data))
	if err != nil || "image/"+format != typ {
		return fmt.Errorf("it isn't a %s", typ)
	}
	if cfg.Width <= 0 || cfg.Height <= 0 || int64(cfg.Width)*int64(cfg.Height) > MaxPixels {
		return fmt.Errorf("it's %dx%d, over %d megapixels", cfg.Width, cfg.Height, MaxPixels/1_000_000)
	}
	select {
	case decodeSlots <- struct{}{}:
	case <-ctx.Done():
		return ctx.Err()
	}
	defer func() { <-decodeSlots }()
	if typ == "image/png" {
		_, err = stdpng.Decode(bytes.NewReader(data))
	} else {
		_, err = jpeg.Decode(bytes.NewReader(data))
	}
	if err != nil {
		return fmt.Errorf("it doesn't decode: %v", err)
	}
	return nil
}

var errAfterEnd = errors.New("there's data after the image")

// pngLayout walks a PNG's chunks to IEND, checking each CRC, and returns
// how many bytes its ancillary chunks hold.
func pngLayout(data []byte) (int, error) {
	if !bytes.HasPrefix(data, []byte("\x89PNG\r\n\x1a\n")) {
		return 0, errors.New("it isn't a PNG")
	}
	meta := 0
	for pos := 8; ; {
		if len(data)-pos < 12 {
			return 0, errors.New("it ends before IEND")
		}
		n := int(binary.BigEndian.Uint32(data[pos:]))
		if n > len(data)-pos-12 {
			return 0, errors.New("a chunk runs past the end")
		}
		body := data[pos+4 : pos+8+n]
		if crc32.ChecksumIEEE(body) != binary.BigEndian.Uint32(data[pos+8+n:]) {
			return 0, errors.New("a chunk's CRC is wrong")
		}
		typ := string(body[:4])
		// Lowercase first letter: ancillary, text and the like.
		if typ[0]&0x20 != 0 {
			meta += n
		}
		pos += 12 + n
		if typ == "IEND" {
			if pos != len(data) {
				return 0, errAfterEnd
			}
			return meta, nil
		}
	}
}

// jpegLayout walks a JPEG's segments and entropy-coded data to EOI, and
// returns how many bytes its APPn and COM segments hold.
func jpegLayout(data []byte) (int, error) {
	if !bytes.HasPrefix(data, []byte{0xff, 0xd8}) {
		return 0, errors.New("it isn't a JPEG")
	}
	meta := 0
	pos := 2
	for {
		if pos >= len(data) || data[pos] != 0xff {
			return 0, errors.New("it ends before EOI")
		}
		for pos < len(data) && data[pos] == 0xff {
			pos++
		}
		if pos >= len(data) {
			return 0, errors.New("it ends before EOI")
		}
		m := data[pos]
		pos++
		switch {
		case m == 0xd9:
			if pos != len(data) {
				return 0, errAfterEnd
			}
			return meta, nil
		case m == 0x01 || (m >= 0xd0 && m <= 0xd7):
			continue
		}
		if len(data)-pos < 2 {
			return 0, errors.New("a segment runs past the end")
		}
		n := int(binary.BigEndian.Uint16(data[pos:]))
		if n < 2 || n > len(data)-pos {
			return 0, errors.New("a segment runs past the end")
		}
		if (m >= 0xe0 && m <= 0xef) || m == 0xfe {
			meta += n
		}
		pos += n
		if m != 0xda {
			continue
		}
		// After a scan header, entropy-coded data runs to the next marker
		// that isn't a stuffed 0xff00 or a restart.
		for pos < len(data)-1 {
			if data[pos] == 0xff {
				next := data[pos+1]
				if next != 0x00 && next != 0xff && (next < 0xd0 || next > 0xd7) {
					break
				}
				if next == 0xff {
					pos++
					continue
				}
				pos += 2
				continue
			}
			pos++
		}
		if pos >= len(data)-1 {
			return 0, errors.New("it ends before EOI")
		}
	}
}
