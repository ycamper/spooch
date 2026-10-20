// Package token makes and checks the random strings the service hands
// out or receives: device tokens, and the ids and secrets in share links.
package token

import (
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/hex"
	"math/big"
	"strings"
)

const alphabet = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"

// IDLen and SecretLen are the lengths of a share link's two halves: a
// public id, then a 128-bit secret (22 base62 characters).
const (
	IDLen     = 10
	SecretLen = 22
)

// DevicePrefix starts every device token, so a leaked one is easy to spot.
const DevicePrefix = "spd_"

// Random returns n base62 characters from the system's secure source.
func Random(n int) string {
	var b strings.Builder
	max := big.NewInt(int64(len(alphabet)))
	for range n {
		i, err := rand.Int(rand.Reader, max)
		if err != nil {
			panic("token: no randomness: " + err.Error())
		}
		b.WriteByte(alphabet[i.Int64()])
	}
	return b.String()
}

// NewDevice returns a fresh device token.
func NewDevice() string {
	return DevicePrefix + Random(32)
}

// Hash is the hex SHA-256 the service stores in place of a token or secret.
func Hash(s string) string {
	sum := sha256.Sum256([]byte(s))
	return hex.EncodeToString(sum[:])
}

// Same compares two hex hashes in constant time.
func Same(a, b string) bool {
	return subtle.ConstantTimeCompare([]byte(a), []byte(b)) == 1
}

// IsBase62 reports whether s is n base62 characters.
func IsBase62(s string, n int) bool {
	if len(s) != n {
		return false
	}
	for i := 0; i < len(s); i++ {
		if !strings.ContainsRune(alphabet, rune(s[i])) {
			return false
		}
	}
	return true
}

// IsHash reports whether s is a hex SHA-256 as Hash writes it: lowercase,
// so the two compare equal.
func IsHash(s string) bool {
	if len(s) != 64 {
		return false
	}
	for i := 0; i < len(s); i++ {
		if !strings.ContainsRune("0123456789abcdef", rune(s[i])) {
			return false
		}
	}
	return true
}

// SplitLink splits a link's path part into its id and secret, if it is
// one.
func SplitLink(s string) (id, secret string, ok bool) {
	if len(s) != IDLen+SecretLen {
		return "", "", false
	}
	id, secret = s[:IDLen], s[IDLen:]
	return id, secret, IsBase62(id, IDLen) && IsBase62(secret, SecretLen)
}
