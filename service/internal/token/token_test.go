package token

import (
	"strings"
	"testing"
)

func TestRandomIsBase62OfTheRightLength(t *testing.T) {
	s := Random(IDLen)
	if !IsBase62(s, IDLen) {
		t.Fatalf("%q isn't %d base62 characters", s, IDLen)
	}
	if Random(SecretLen) == Random(SecretLen) {
		t.Fatal("two secrets came out the same")
	}
}

func TestDeviceTokensHaveThePrefix(t *testing.T) {
	d := NewDevice()
	if !strings.HasPrefix(d, DevicePrefix) || len(d) != len(DevicePrefix)+32 {
		t.Fatalf("bad device token %q", d)
	}
}

func TestLinksSplitIntoIDAndSecret(t *testing.T) {
	id, secret := Random(IDLen), Random(SecretLen)
	gotID, gotSecret, ok := SplitLink(id + secret)
	if !ok || gotID != id || gotSecret != secret {
		t.Fatalf("split %q into %q, %q, %v", id+secret, gotID, gotSecret, ok)
	}
	for _, bad := range []string{"", "short", id + secret + "x", id + secret[:21] + "!"} {
		if _, _, ok := SplitLink(bad); ok {
			t.Fatalf("%q split as a link", bad)
		}
	}
}

func TestHashesCompare(t *testing.T) {
	h := Hash("secret")
	if !IsHash(h) || !Same(h, Hash("secret")) || Same(h, Hash("other")) {
		t.Fatal("hash comparison is wrong")
	}
	if IsHash("nothex") || IsHash(strings.ToUpper(h)) {
		t.Fatal("accepted a non-hash, or uppercase that would never match")
	}
}
