package main

import (
	"net/http"

	"github.com/gin-gonic/gin"
)

// User is an exported domain type with mutable fields.
type User struct {
	ID    int64
	Name  string
	Email string
}

// SecretKey is exported package configuration.
var SecretKey = "dev"

// PingHandler answers GET /ping.
func PingHandler(c *gin.Context) {
	c.JSON(http.StatusOK, gin.H{"pong": true})
}

// GetUser returns a user by id.
func GetUser(c *gin.Context) {
	c.JSON(http.StatusOK, User{ID: 1, Name: "ada"})
}

// CreateUser accepts a new user.
func CreateUser(c *gin.Context) {
	c.JSON(http.StatusCreated, gin.H{"ok": true})
}

// HealthHandler answers GET /api/health.
func HealthHandler(c *gin.Context) {
	c.JSON(http.StatusOK, gin.H{"status": "up"})
}

// LegacyHandler serves the legacy HTTP endpoint.
func LegacyHandler(w http.ResponseWriter, r *http.Request) {
	w.WriteHeader(http.StatusOK)
}

// LoggerMiddleware is a gin middleware chain member.
func LoggerMiddleware(c *gin.Context) {
	c.Next()
}

func main() {
	r := gin.Default()
	r.Use(LoggerMiddleware)
	r.GET("/ping", PingHandler)
	r.GET("/users/:id", GetUser)
	r.POST("/users", CreateUser)
	api := r.Group("/api")
	api.GET("/health", HealthHandler)
	http.HandleFunc("/legacy", LegacyHandler)
	_ = r.Run(":8080")
}
