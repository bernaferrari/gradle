package example;

import com.google.common.collect.ImmutableList;

public class UseGuava {
    public int size() {
        return ImmutableList.of("a", "b").size();
    }
}
