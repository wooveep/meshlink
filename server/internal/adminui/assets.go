package adminui

import (
	"embed"
	"io/fs"
	"mime"
	"net/http"
	"path"
	"strings"
)

//go:embed dist dist/*
var embedded embed.FS

func Handler(basePath string) http.Handler {
	sub, err := fs.Sub(embedded, "dist")
	if err != nil {
		return http.NotFoundHandler()
	}

	base := strings.TrimRight(basePath, "/")
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		trimmed := strings.TrimPrefix(r.URL.Path, base)
		trimmed = strings.TrimPrefix(trimmed, "/")
		if trimmed == "" {
			serveEmbeddedFile(w, sub, "index.html")
			return
		}

		cleaned := path.Clean(trimmed)
		if cleaned == "." || cleaned == "/" {
			serveEmbeddedFile(w, sub, "index.html")
			return
		}
		if hasEmbeddedFile(sub, cleaned) {
			serveEmbeddedFile(w, sub, cleaned)
			return
		}

		serveEmbeddedFile(w, sub, "index.html")
	})
}

func hasEmbeddedFile(files fs.FS, name string) bool {
	info, err := fs.Stat(files, name)
	if err != nil {
		return false
	}
	return !info.IsDir()
}

func serveEmbeddedFile(w http.ResponseWriter, files fs.FS, name string) {
	data, err := fs.ReadFile(files, name)
	if err != nil {
		http.Error(w, "not found", http.StatusNotFound)
		return
	}

	if contentType := mime.TypeByExtension(path.Ext(name)); contentType != "" {
		w.Header().Set("Content-Type", contentType)
	}
	_, _ = w.Write(data)
}
