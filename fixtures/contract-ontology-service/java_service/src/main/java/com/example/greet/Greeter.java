package com.example.greet;

/** Extension point: greeting providers. Declared in its own file so
 * implementing classes register against a non-local surface. */
public interface Greeter {
    String greet(String name);
}
