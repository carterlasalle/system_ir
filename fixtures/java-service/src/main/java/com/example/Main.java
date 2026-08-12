package com.example;

/** Application entrypoint: processes orders through the service. */
public class Main {

    public static void main(String[] args) {
        Service service = new Service();
        service.process("order-42");
    }
}
