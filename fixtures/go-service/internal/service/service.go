package service

import (
	"database/sql"
	"fmt"
)

// Store persists orders.
type Store struct {
	db *sql.DB
}

// NewStore opens the order database.
func NewStore() *Store {
	db, err := sql.Open("sqlite3", "orders.db")
	if err != nil {
		panic("cannot open database")
	}
	return &Store{db: db}
}

// Save writes an order.
func (s *Store) Save(order string) error {
	_, err := s.db.Exec("INSERT INTO orders (name) VALUES (?)", order)
	if err != nil {
		return fmt.Errorf("store write failed: %w", err)
	}
	return nil
}

// Sync flushes pending writes.
func (s *Store) Sync() error {
	_, err := s.db.Exec("UPDATE orders SET synced = 1")
	return err
}
