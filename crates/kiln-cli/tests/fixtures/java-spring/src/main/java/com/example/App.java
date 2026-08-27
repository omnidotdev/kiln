package com.example;

import org.springframework.boot.SpringApplication;
import org.springframework.boot.autoconfigure.SpringBootApplication;
import org.springframework.web.bind.annotation.GetMapping;
import org.springframework.web.bind.annotation.RestController;

// A minimal Spring Boot app. The Gradle build emits both app-0.0.1.jar (boot)
// and app-0.0.1-plain.jar; the provider must select the executable one.
@SpringBootApplication
@RestController
public class App {
    public static void main(String[] args) {
        SpringApplication.run(App.class, args);
    }

    @GetMapping("/")
    public String ok() {
        return "ok\n";
    }
}
