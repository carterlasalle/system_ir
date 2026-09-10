// trace:exempt reason=test-data
package main

import "github.com/nats-io/nats.go"

func onOrder(msg *nats.Msg) {}

func consumeOrders(nc *nats.Conn) {
	nc.Subscribe("orders.created", onOrder)
}

func main() {}
