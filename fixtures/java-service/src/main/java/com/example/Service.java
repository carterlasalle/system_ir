package com.example;

import java.sql.Connection;
import java.sql.DriverManager;
import java.sql.PreparedStatement;
import java.sql.SQLException;

/** Order service: persists orders to the JDBC store and fans out. */
public class Service {

    private final Connection connection;

    public Service() {
        try {
            this.connection = DriverManager.getConnection("jdbc:sqlite:orders.db");
        } catch (SQLException e) {
            throw new RuntimeException("db unavailable", e);
        }
    }

    /** Process an order: persist it, then fan out to consumers. */
    @Retryable(maxAttempts = 3, backoff = @Backoff(delay = 100))
    public void process(String orderId) {
        try {
            this.storeOrder(orderId);
            this.fanout(orderId);
        } catch (Exception e) {
            this.fallback(orderId);
        }
    }

    /** Write the order row to the JDBC store. */
    public void storeOrder(String orderId) {
        try (PreparedStatement ps = this.connection.prepareStatement(
                "INSERT INTO orders (id) VALUES (?)")) {
            ps.setString(1, orderId);
            ps.executeUpdate();
        } catch (SQLException e) {
            throw new RuntimeException("store failed", e);
        }
    }

    /** Fan out to downstream consumers. */
    public void fanout(String orderId) {
        System.out.println("fanout " + orderId);
    }

    /** Fallback path when processing fails. */
    public void fallback(String orderId) {
        System.out.println("fallback " + orderId);
    }
}
