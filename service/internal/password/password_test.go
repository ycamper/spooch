package password

import "testing"

func TestHashesCheckOnlyTheirPassword(t *testing.T) {
	h, err := Hash("correct horse battery")
	if err != nil {
		t.Fatal(err)
	}
	h2, _ := Hash("correct horse battery")
	if h == h2 {
		t.Fatal("two hashes of one password are the same: no salt")
	}
	if !Check("correct horse battery", h) || !Check("correct horse battery", h2) {
		t.Fatal("the password doesn't check")
	}
	for _, wrong := range []string{"", "correct horse batter", "Correct horse battery"} {
		if Check(wrong, h) {
			t.Fatalf("%q checks", wrong)
		}
	}
	if Check("x", "$argon2id$nonsense") || Check("x", "") {
		t.Fatal("a broken hash checks")
	}
}
