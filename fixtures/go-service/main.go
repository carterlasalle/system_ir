package main

import (
	"fmt"

	svc "go-service/internal/service"
)

func main() {
	store := svc.NewStore()
	if err := store.Save("order-1"); err != nil {
		fmt.Println("save failed:", err)
	}
	if err := store.Sync(); err != nil {
		panic("sync failed")
	}
}
