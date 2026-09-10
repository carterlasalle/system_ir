// trace:exempt reason=test-data
package main

import "github.com/nats-io/nats.go"

func publishOrder(producer *nats.Conn) {
	producer.Publish("orders.created", []byte("{}"))
}

func main() {}
