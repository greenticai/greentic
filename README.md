IMPORTANT: The next-generation Greentic will soon launch. This repo is just a proof of concept. Go to [Geentic.ai](https://greentic.ai) and join the waiting list to get access to the open source digital worker platform that will scale to thousands of workers per employee...
<table style="border: none;">
<tr>
<td><img src="assets/greentic-logo-very-small.png" alt="Greentic.AI Logo" width="150"></td>
<td><h1>Greentic.AI 🚀</h1><br><strong>Build armies of digital workers:</strong> fast, secure, and extendable. Automate anything using Wasm tools, channels, agents, and flows.</td>
</tr>
</table>
Now with: <strong>intelligent agents</strong> and <strong>processes</strong>!

---

![Telegram Weather Bot](https://greentic.ai/assets/telegram-weather-bot.gif)

Greentic.ai is now at version 0.2.0, offering a growing store with free flows, plugins, and tools to get you started. You can easily build your own flows, tools, and plugins, including those that connect to APIs without requiring authentication or API keys. Support for OAuth integrations is coming in v0.3.0, and full Cloud deployment will be available in v0.4.0.

Looking ahead, the [vision for v1.0.0](./docs/VISION.md) is ambitious: imagine simply messaging via WhatsApp, Teams, Slack, or Telegram to request a digital worker—Greentic.ai will create it automatically based on your request, just like ChatGPT.

Discover how [Greentic.ai enables revenue oppotunities for partners](./docs/VISION.md) and be part of the future of intelligent automation.

---

## 📋 Table of Contents

1. [Introduction](#introduction)
2. [What is a Digital Worker?](#wat-is-a-digital-worker)
3. [Key Concepts](#key-concepts)
   - [Tools (MCP in Wasm)](#tools-mcp-in-wasm)
   - [Channels](#channels)
   - [Processes](#processes)
   - [Agents](#agents)
4. [Getting Started](#getting-started)
5. [Quick Flow Example (YAML)](#quick-flow-example-yaml)
6. [Controlling Flows, Channels & Tools](#controlling-flows-channels--tools)
7. [Coming Soon](#coming-soon)
8. [Need Custom Agentic Automation?](#need-custom-agentic-automation)
9. [Contributing](#contributing)
10. [License](#license)

---

## 📝 Introduction

Greentic.AI is an open-source platform designed to let you build, deploy, and manage digital workers at lightning speed.

- **Fastest** runtime with zero cold-starts for WebAssembly tools.
- **Extendable** architecture: plug in your own channels, tools, agents and processes, all defined in an easy to understand text-based flow.
- **Secure** by design: tools are sandboxed inside Wasm allowing securely running untrusted third-party MCP tools.
- **Observability** via OpenTelemetry integrations

---

## 🤖 What is a Digital Worker?

A **Digital Worker** is a flow that acts autonomously and intelligently to handle a complete task, from end to end.

It:

- Listens for messages (via **Channels** like Telegram or Slack)
- Extracts meaning or decisions (via **Agents**, powered by LLMs)
- Calls APIs or executes functions (via **Tools** written in Wasm)
- Handles control logic (via **Processes** like retries, conditionals, loops)

Flows link these components into one cohesive automation. Your digital workers are secure, modular, and language-agnostic.

---

## 🔑 Key Concepts

### Tools (MCP in Wasm)

- **MCP** (Model-Context Protocol) modules compile to WebAssembly.
- Each tool can define its own actions, inputs, outputs, and run logic securely.
- Tools live in `tools/` and are called by the flows.

👉 [Learn how to build MCP Tools](./docs/TOOLS.md)

### Channels

- **Channels** allow flows to send/receive messages to/from the outside world.
- Examples: Telegram, Slack, Email, HTTP Webhooks.

👉 [How to build Channel Plugins](./docs/PLUGIN.md)

### Processes

- **Processes** are a collection of builtIn processes and soon extendable via Wasm.
- Debug: allows you to easily understand the output of the previous flow nodes.
- Script: create a script in Rhai to programme logic.
- Template: a Handlebars-based template processor for rending string output.
- QA: A dynamic, multi-question form-like process with optional validation, LLM user assistance and routing.   
- Defined declaratively in YAML.

👉 [Learn more about Processes](./docs/PROCESSES.md)

### Agents

- **Agents** are LLM-powered nodes capable of autonomous decision-making.
- `openai` calls the OpenAI Chat Completions API. Provide an `OPENAI_KEY` secret and optionally `OPENAI_URL` to target a compatible endpoint.
- `ollama` connects to a local Ollama server for chat, generation, and embeddings. Configure the model, mode, and tools per flow.
- Coming Soon: richer memory, tool orchestration, and goal-following behaviours.

👉 [Learn more about Agents](./docs/AGENTS.md)

---

## 🦀 Prerequisites: Install Rust

To build and run this project, you need to have [Rust](https://www.rust-lang.org/tools/install) installed.

If you don’t have Rust yet, the easiest way is via `rustup`:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```
You might have to restart the terminal to get Rust to work. Test it via 'cargo --version'
---

## 🚀 Getting Started: Install Greentic

Install Greentic.AI via:

```bash
cargo install greentic
```

### 🔧 Initialise your environment

The first time you use Greentic, run:

```bash
greentic init
```

This will:

- Create the Greentic configuration directories
- Register your user and generate a `GREENTIC_TOKEN`
- Allow you to pull flows, channels, tools, etc. from [greenticstore.com](https://greenticstore.com)
- Will give an error about the TELEGRAM_TOKEN and WEATHERAPI_KEY not being set. Read about how to create a 
[Telegram bot](./docs/TELEGRAM.md) and get your free WEATHERAPI_KEY at [https://www.weatherapi.com/](https://www.weatherapi.com/)

---

### 🌦️ Example: Telegram Weather Bot

Pull your first flow: (greentic init does this for you already)

```bash
greentic flow pull weather_bot_telegram.ygtc
```

Then:

1. [Create and configure a Telegram bot](./docs/TELEGRAM.md), and add your token:

   ```bash
   greentic secrets add TELEGRAM_TOKEN <your_token>
   ```

2. [Sign up to WeatherAPI](https://www.weatherapi.com/signup.aspx) and add your free API key:

   ```bash
   greentic secrets add WEATHERAPI_KEY <your_key>
   ```

3. (Optional) To enable AI-powered queries like *“What’s the weather in London tomorrow?”*:

   - [Install Ollama](https://ollama.com/download)
   - Pull the model:

     ```bash
     ollama pull gemma:instruct
     ```

---

### ▶️ Run the bot

```bash
greentic run
```

You should now have a fully working **Telegram Weather Bot**.

---

## 🛠️ Creating Your Own Flows

To deploy your own flows:

```bash
greentic flow deploy <file>.ygtc
```

To start a flow:

```bash
greentic flow start <flow_id>
```

---

## 🛠 Quick Flow Example (YAML)

```yaml
id: weather_bot
title: Get your weather prediction
description: >
  This flow shows how you can combine either a fixed question and answer process
  with an AI fallback if the user is not answering the questions correctly.
channels:
  - telegram  
nodes:
  # 1) Messages come in via Telegram
  telegram_in:
    channel: telegram
    in: true

   # 2) QA node: ask for the city and fallback to the OllamaAgent if more than 3 words are used
  extract_city:
    qa:
      welcome_template: "Hi there! Let's get your weather forecast."
      questions:
        - id: q_location
          prompt: "👉 What location would you like a forecast for?"
          answer_type: text
          state_key: q
          max_words: 3
      fallback_agent:
        type: ollama
        model: gemma:instruct
        task: |
          The user wants the weather forecast. Find out for which city or location they want the weather and
          assign this to a state value named `q`. If they mention the days, assign the number to a state value named `days`, 
          otherwise use `3` for `days`.
          If you are unsure about the place (`q`), ask the user to clarify where they want the weather forecast for.
      routing:
        - to: forecast_weather
  # 3) “forecast_weather”: the Weather API tool, using the JSON from parse_request.
  forecast_weather:
    tool:
      name: weather_api
      action: forecast_weather
    parameters:
      q: "{{extract_city.payload.city}}"
      days: 3

  # 4) “weather_template”: format the weather API’s JSON into a friendly sentence.
  weather_out_template:
    template: |
      🌤️ Weather forecast for {{payload.location.name}}:

      {{#each payload.forecast.forecastday}}
      📅 Day {{@indexPlusOne}} ({{this.date}}):
      • High: {{this.day.maxtemp_c}}°C
      • Low: {{this.day.mintemp_c}}°C
      • Condition: {{this.day.condition.text}}
      • Rain? {{#if (eq this.day.daily_will_it_rain 1)}}Yes{{else}}No{{/if}}

      {{/each}}

  # 5) “telegram_out”: send the forecast back to Telegram.
  telegram_out:
    channel: telegram
    out: true

connections:
  telegram_in:
    - extract_city

  extract_city:
    - forecast_weather

  forecast_weather:
    - weather_out_template

  weather_out_template:
    - telegram_out 
```

---

## ⚙️ Controlling Flows, Channels & Tools

```bash
# Validate a flow before deploying. Afterwards you can start/stop the flow
greentic flow validate <file>.ygtc 
greentic flow deploy <file>.ygtc
greentic flow start <flow-id>
greentic flow stop <flow-id>
```

---

## ⚡ Fast-start Snapshots

Snapshots capture the current runtime state (flows, tools, channels, processes, configs, secrets).

### Create a snapshot

```bash
greentic snapshot export --file out.gtc
```

### Validate a snapshot locally

```bash
greentic snapshot validate --file out.gtc --json
```

### Take & validate the current workspace in one go

```bash
greentic snapshot validate --take --json
```

### ValidationPlan schema

`greentic snapshot validate` returns a `ValidationPlan` JSON payload:

| Field | Type | Description |
|---|---|---|
| `ok` | bool | All requirements satisfied |
| `summary` | string | Human-readable summary |
| `missing_tools` | Vec<Req> | Required tools missing |
| `missing_channels` | Vec<Req> | Required channels missing |
| `missing_processes` | Vec<Req> | Required processes missing |
| `missing_agents` | Vec<Req> | Required agents missing |
| `missing_configs` | Vec<ConfigReq> | Config entries to set |
| `missing_secrets` | Vec<SecretReq> | Secrets to provide |
| `suggested_commands` | SuggestedCommands | Helper commands (install/config/secret) |

Each `Req` contains `{ id, version_req }`. `ConfigReq` and `SecretReq` include an `owner` (`kind` = tool/channel/process/agent) and optional `description`.

### Example response with missing items

```json
{
  "ok": false,
  "summary": "2 items missing (1 tool, 1 secret)",
  "missing_tools": [
    { "id": "mcp.weather", "version_req": null }
  ],
  "missing_channels": [],
  "missing_processes": [],
  "missing_agents": [],
  "missing_configs": [],
  "missing_secrets": [
    {
      "key": "WEATHERAPI_KEY",
      "owner": { "kind": "tool", "id": "weather_api" },
      "description": "WeatherAPI.com API key for weather_api tool"
    }
  ],
  "suggested_commands": {
    "install": [
      "greentic store install tool mcp.weather@latest"
    ],
    "config": [],
    "secret": [
      "greentic secrets add WEATHERAPI_KEY <VALUE>  # tool:weather_api"
    ]
  }
}
```

### Exit codes

* `0` – plan is OK
* `2` – items missing (ideal for CI/CD)

---

## 🔭 Coming Soon

---

## 🔭 Coming Soon

- v0.3.0 oAuth MCP Tools - connect to any SaaS
- v0.4.0 Serverless Cloud deployment of flows - greentic deploy <flow>

Roadmap:
- More Agentic: memory persistence, vector databases, A2A,...
- AI Flow Designer
- Flow, Tools, Channels & Processes marketplace

---

## 📬 Need Custom Agentic Automation?

Have a specific use-case or need expert help?\
Please fill out our form: [Agentic Automation Inquiry](https://forms.gle/h17SdjoUxozJf6XA6)

---

## 🤝 Contributing

We are actively looking for contributors and welcome contributions of all kinds!

- Bug reports 🐞
- Feature requests 🎉
- Code & documentation PRs 📝

1. Fork the repo
2. Create a feature branch
3. Open a PR against `main`

See [CONTRIBUTING.md](./docs/CONTRIBUTING.md) for full guidelines.

---

## 📄 License

Distributed under the **MIT License**. See [LICENSE](./LICENSE) for details.

---

Thank you for checking out **Greentic.AI**—let’s build the future of automation together! 🚀
