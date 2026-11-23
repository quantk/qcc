// The built-in modules. Each one is self-contained: to add a feature, write a
// module that registers its contributions and list it here.

import type { UiModule } from "../ui/registry";
import { automationsModule } from "./automations";
import { changesModule } from "./changes";
import { contextModule } from "./context";
import { forkModule } from "./fork";
import { insightsModule } from "./insights";
import { lineageModule } from "./lineage";
import { limitsModule } from "./limits";
import { liveModule } from "./live";
import { mcpModule } from "./mcp";
import { metricsModule } from "./metrics";
import { navigationModule } from "./navigation";
import { newThreadModule } from "./new-thread";
import { notificationsModule } from "./notifications";
import { openInEditorModule } from "./open-in-editor";
import { overviewModule } from "./overview";
import { pluginCommandsModule } from "./plugin-commands";
import { pluginUiModule } from "./plugin-ui";
import { projectsModule } from "./projects";
import { settingsModule } from "./settings";
import { skillsModule } from "./skills";
import { threadControlsModule } from "./thread-controls";
import { threadMenuModule } from "./thread-menu";
import { timelineModule } from "./timeline";

export const builtinModules: UiModule[] = [
  overviewModule,
  liveModule,
  timelineModule,
  changesModule,
  lineageModule,
  forkModule,
  threadControlsModule,
  threadMenuModule,
  openInEditorModule,
  newThreadModule,
  navigationModule,
  projectsModule,
  pluginUiModule,
  pluginCommandsModule,
  mcpModule,
  automationsModule,
  skillsModule,
  contextModule,
  insightsModule,
  metricsModule,
  limitsModule,
  notificationsModule,
  settingsModule,
];
