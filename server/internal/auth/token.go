package auth

import "errors"

var ErrInvalidToken = errors.New("invalid bootstrap token")

type TokenValidator struct {
	expected string
}

func NewTokenValidator(expected string) *TokenValidator {
	return &TokenValidator{expected: expected}
}

func (v *TokenValidator) Validate(token string) error {
	if token == "" || token != v.expected {
		return ErrInvalidToken
	}
	return nil
}
