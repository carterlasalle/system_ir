package com.example.surface;

import java.io.IOException;
import java.util.List;

public interface Repository<T> {
    List<T> find(String owner, int limit) throws IOException;
}

public class IncidentService<T> implements Repository<T> {
    public List<T> findIncidents(
        String owner,
        int limit
    ) throws IOException {
        return List.of();
    }
}
