// Contract-ontology go half: the encoding/json interface pair
// (MarshalJSON/UnmarshalJSON) around a type — a serialization contract.

package ontology

import (
	"encoding/json"
	"fmt"
)

// User is a model with the json.Marshaler/Unmarshaler pair.
type User struct {
	Name string
}

// MarshalJSON implements json.Marshaler.
func (u User) MarshalJSON() ([]byte, error) {
	return json.Marshal(map[string]string{"name": u.Name})
}

// UnmarshalJSON implements json.Unmarshaler.
func (u *User) UnmarshalJSON(data []byte) error {
	var m map[string]string
	if err := json.Unmarshal(data, &m); err != nil {
		return err
	}
	u.Name = m["name"]
	return nil
}

// NewUser is a package factory (public-api surface).
func NewUser(name string) User {
	return User{Name: name}
}

// String is a plain method, not a serialization pair.
func (u User) String() string {
	return fmt.Sprintf("%s", u.Name)
}
