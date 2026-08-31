package com.example;

import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;
import java.lang.annotation.Target;

/** Backoff policy between retry attempts. */
@Retention(RetentionPolicy.RUNTIME)
@Target({})
public @interface Backoff {
    long delay() default 1000;
}
