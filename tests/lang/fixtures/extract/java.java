package fixtures;

import java.util.List;

/** Class docs mention docOnlyJava and should not become code. */
public class GoldenWidget {
    /** Constructor docs mention docOnlyJava. */
    public GoldenWidget() {
    }

    /** Method docs mention docOnlyJava. */
    public String render(List<String> labels) {
        return formatWidget(labels.get(0));
    }

    private String formatWidget(String name) {
        return name.trim();
    }
}
