package observability

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"time"
)

type HTTPStatusClient struct {
	client *http.Client
}

func NewHTTPStatusClient(timeout time.Duration) *HTTPStatusClient {
	if timeout <= 0 {
		timeout = 2 * time.Second
	}
	return &HTTPStatusClient{
		client: &http.Client{Timeout: timeout},
	}
}

func (c *HTTPStatusClient) GetJSON(ctx context.Context, url string, target interface{}) error {
	if url == "" {
		return fmt.Errorf("status url is required")
	}

	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return fmt.Errorf("build status request: %w", err)
	}

	resp, err := c.client.Do(req)
	if err != nil {
		return fmt.Errorf("fetch status url %s: %w", url, err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("fetch status url %s: unexpected status %d", url, resp.StatusCode)
	}

	if err := json.NewDecoder(resp.Body).Decode(target); err != nil {
		return fmt.Errorf("decode status response: %w", err)
	}
	return nil
}
