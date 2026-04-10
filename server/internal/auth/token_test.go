package auth

import "testing"

func TestTokenValidatorValidate(t *testing.T) {
	validator := NewTokenValidator("meshlink-dev-token")

	if err := validator.Validate("meshlink-dev-token"); err != nil {
		t.Fatalf("expected token to validate: %v", err)
	}

	if err := validator.Validate("wrong-token"); err == nil {
		t.Fatal("expected invalid token error")
	}
}
