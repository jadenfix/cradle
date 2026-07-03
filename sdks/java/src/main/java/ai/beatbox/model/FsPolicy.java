package ai.beatbox.model;

import com.fasterxml.jackson.annotation.JsonInclude;
import java.util.List;

/** Filesystem policy: an optional workspace and a set of mounts. */
@JsonInclude(JsonInclude.Include.NON_NULL)
public record FsPolicy(String workspace, List<Mount> mounts) {
}
