package example;

import java.nio.file.Files;
import java.nio.file.Path;

public final class Tool {
    private Tool() {
    }

    public static void main(String[] args) throws Exception {
        if (args.length != 2) {
            throw new IllegalArgumentException("expected output path and token");
        }
        Path output = Path.of(args[0]);
        Files.createDirectories(output.getParent());
        Files.writeString(output, "javaexec:" + args[1] + System.lineSeparator());
    }
}
