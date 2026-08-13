package com.example.greet;

/** An implementation of the Greeter extension point, with a
 * serializer/deserializer pair (toJson/fromJson) around the class. */
public class GreetingImpl implements Greeter {

    private final String prefix = "Hello";

    @Override
    public String greet(String name) {
        return this.prefix + ", " + name;
    }

    /** Serialize side of the pair. */
    public String toJson() {
        return "{\"prefix\":\"" + this.prefix + "\"}";
    }

    /** Deserialize side of the pair. */
    public static GreetingImpl fromJson(String json) {
        return new GreetingImpl();
    }
}
