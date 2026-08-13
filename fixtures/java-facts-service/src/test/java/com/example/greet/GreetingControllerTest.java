package com.example.greet;

import static org.junit.Assert.assertEquals;

import org.junit.Before;
import org.junit.BeforeClass;
import org.junit.Rule;
import org.junit.Test;
import org.junit.rules.TemporaryFolder;

/** Verifies the greeting surface. */
public class GreetingControllerTest {

    @Rule
    public TemporaryFolder tmp = new TemporaryFolder();

    @BeforeClass
    public static void setupAll() {
    }

    @Before
    public void setUp() {
    }

    @Test
    public void greetReturnsHello() {
        assertEquals("Hello", new GreetingService().message());
    }
}
