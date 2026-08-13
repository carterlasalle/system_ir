// guava-family: static factories (`of`, `builder()`) and a fluent
// `Builder.withX() { return this; }` chain; static fields as state.

package com.example.common;

/** Immutable list with static factories, guava-style. */
public final class ImmutableList<E> {
    public static int DEFAULT_CAPACITY = 16;
    public static final String PACKAGE = "guava";

    private final Object[] elements;

    private ImmutableList(Object[] elements) {
        this.elements = elements;
    }

    public static <E> ImmutableList<E> of(E e1) {
        return new ImmutableList<E>(new Object[] { e1 });
    }

    public static <E> ImmutableList<E> of(E e1, E e2) {
        return new ImmutableList<E>(new Object[] { e1, e2 });
    }

    public static <E> Builder<E> builder() {
        return new Builder<E>();
    }

    public static void sort() {
        // not a factory
    }

    /** Fluent builder for ImmutableList. */
    public static final class Builder<E> {
        private final java.util.ArrayList<E> acc = new java.util.ArrayList<E>();

        public Builder<E> withTag(String tag) {
            return this;
        }

        public Builder<E> add(E element) {
            this.acc.add(element);
            return this;
        }

        public ImmutableList<E> build() {
            return new ImmutableList<E>(this.acc.toArray());
        }
    }
}
