// The built-in modules. Each one is self-contained: to add a feature, write a
// module that registers its contributions and list it here.

import type { UiModule } from "../ui/registry";
import { changesModule } from "./changes";
import { newThreadModule } from "./new-thread";
import { overviewModule } from "./overview";
import { pluginUiModule } from "./plugin-ui";
import { threadControlsModule } from "./thread-controls";
import { timelineModule } from "./timeline";

export const builtinModules: UiModule[] = [overviewModule, timelineModule, changesModule, threadControlsModule, newThreadModule, pluginUiModule];
