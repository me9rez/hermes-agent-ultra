---
[meta]
id = "computer-use"
display_name_key = "vertical.computer_use.name"
description_key = "vertical.computer_use.description"
icon = "desktop"
category = "productivity"
order = 300
task_category = "Reasoning"

[provider]
default_tier = "smart"

[provider.tier_overrides]
smart = "gpt-5-relay"
economic = "qwen-vl-max"
local = "N/A"

[persona]
strategy = "auto_blend"

[[persona.blocks]]
kind = "instruction"
follow_user_locale = false
variants = { en = "persona.en.md", "zh-CN" = "persona.zh-CN.md" }

[[persona.blocks]]
kind = "output_directive"
follow_user_locale = true
---

# Computer Use

Browser automation via system Chrome/Edge and CDP.

Default tools: `browser_navigate`, `browser_click`, `browser_type`, `browser_screenshot`.
