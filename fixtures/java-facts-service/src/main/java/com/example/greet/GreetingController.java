package com.example.greet;

import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.RequestMapping;
import org.springframework.web.bind.annotation.RestController;

/** REST controller exposing the greeting surface. */
@RestController
@RequestMapping("/api")
public class GreetingController {

    private final GreetingService service;

    public GreetingController(GreetingService service) {
        this.service = service;
    }

    @GetMapping("/greet")
    public String greet() {
        return service.message();
    }

    @GetMapping("/greet/{name}")
    public String greetName(String name) {
        return service.message();
    }
}
