// Package password hashes account passwords with argon2id, in the PHC
// string format, and checks them.
package password

import (
	"crypto/rand"
	"crypto/subtle"
	"encoding/base64"
	"errors"
	"fmt"
	"strings"

	"golang.org/x/crypto/argon2"
)

// The cost: OWASP's argon2id settings (19 MiB, two passes, one lane).
const (
	memory  = 19 * 1024
	passes  = 2
	lanes   = 1
	saltLen = 16
	keyLen  = 32
)

var b64 = base64.RawStdEncoding

// Hash is `p` hashed with a fresh salt, as "$argon2id$v=19$m=...$salt$key".
func Hash(p string) (string, error) {
	salt := make([]byte, saltLen)
	if _, err := rand.Read(salt); err != nil {
		return "", err
	}
	key := argon2.IDKey([]byte(p), salt, passes, memory, lanes, keyLen)
	return fmt.Sprintf("$argon2id$v=%d$m=%d,t=%d,p=%d$%s$%s",
		argon2.Version, memory, passes, lanes, b64.EncodeToString(salt), b64.EncodeToString(key)), nil
}

var errFormat = errors.New("not an argon2id hash")

// Check reports whether `p` is the password `encoded` was made from, at
// the cost it was made with.
func Check(p, encoded string) bool {
	ok, err := check(p, encoded)
	return err == nil && ok
}

func check(p, encoded string) (bool, error) {
	parts := strings.Split(encoded, "$")
	if len(parts) != 6 || parts[1] != "argon2id" {
		return false, errFormat
	}
	var version int
	var m, t uint32
	var l uint8
	if _, err := fmt.Sscanf(parts[2], "v=%d", &version); err != nil || version != argon2.Version {
		return false, errFormat
	}
	if _, err := fmt.Sscanf(parts[3], "m=%d,t=%d,p=%d", &m, &t, &l); err != nil {
		return false, errFormat
	}
	salt, err := b64.DecodeString(parts[4])
	if err != nil {
		return false, err
	}
	want, err := b64.DecodeString(parts[5])
	if err != nil {
		return false, err
	}
	got := argon2.IDKey([]byte(p), salt, t, m, l, uint32(len(want)))
	return subtle.ConstantTimeCompare(got, want) == 1, nil
}

// decoy is a hash nobody's password matches, to check against when there
// is no account, so a wrong name takes as long as a wrong password.
var decoy, _ = Hash("not anyone's password")

// Waste spends as long as Check does, for a name with no account.
func Waste(p string) { Check(p, decoy) }
