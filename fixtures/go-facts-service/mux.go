package main

import (
	"net/http"

	"github.com/gorilla/mux"
)

// Item is an exported domain type.
type Item struct {
	ID   string
	Name string
}

// ListItems handles GET /items.
func ListItems(w http.ResponseWriter, r *http.Request) {
	w.WriteHeader(http.StatusOK)
}

// GetItem handles GET /items/{id}.
func GetItem(w http.ResponseWriter, r *http.Request) {
	w.WriteHeader(http.StatusOK)
}

// SetupMux builds the gorilla router.
func SetupMux() *mux.Router {
	r := mux.NewRouter()
	r.HandleFunc("/items", ListItems).Methods("GET")
	r.HandleFunc("/items/{id}", GetItem)
	return r
}
