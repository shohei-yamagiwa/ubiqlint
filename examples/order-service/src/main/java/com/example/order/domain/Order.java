package com.example.order.domain;

import java.util.ArrayList;
import java.util.List;

public class Order {
    private final List<LineItem> lineItems = new ArrayList<>();

    public void place() {
    }

    public void cancel() {
    }

    public OrderPlaced placeOrder() {
        place();
        return new OrderPlaced("order-1");
    }
}
