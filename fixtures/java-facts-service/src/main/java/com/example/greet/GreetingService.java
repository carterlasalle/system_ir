package com.example.greet;

import org.springframework.stereotype.Service;

/** Greeting business logic. */
@Service
public class GreetingService {

    private final String prefix = "Hello";

    private int count = 0;

    /** Public API surface. */
    public String message() {
        this.count = this.count + 1;
        return this.prefix;
    }

    public String upper(String s) {
        return s.toUpperCase();
    }
}
