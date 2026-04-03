# FunctionGemma iOS Runner — Architecture & Flow

## High-Level Overview

```
┌─────────────────────────────────────────────────────────┐
│                    iOS App (Simulator)                   │
│                                                         │
│  ┌─────────────┐   ┌────────────────┐   ┌───────────┐  │
│  │ ContentView │──▶│ PromptFormatter│──▶│   Model   │  │
│  │   (SwiftUI) │   │                │   │  Manager  │  │
│  │             │◀──│  Parse Output  │◀──│           │  │
│  └─────────────┘   └────────────────┘   └─────┬─────┘  │
│                                               │         │
│                                     ┌─────────▼───────┐ │
│                                     │   MediaPipe     │ │
│                                     │ LLM Inference   │ │
│                                     │    (Native)     │ │
│                                     └─────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

## Detailed Flow

### 1. Model Setup (First Launch)

```
┌──────┐       ┌──────────────┐       ┌──────────────┐       ┌──────────────┐
│ App  │       │ ModelManager │       │  HuggingFace │       │  MediaPipe   │
│Launch│       │              │       │   (Remote)   │       │  LlmInference│
└──┬───┘       └──────┬───────┘       └──────┬───────┘       └──────┬───────┘
   │                  │                      │                      │
   │  downloadModel   │                      │                      │
   │  IfNeeded()      │                      │                      │
   │─────────────────▶│                      │                      │
   │                  │                      │                      │
   │                  │  GET .task file      │                      │
   │                  │  (~271 MB, INT8)     │                      │
   │                  │─────────────────────▶│                      │
   │                  │                      │                      │
   │  state:          │  Stream bytes        │                      │
   │  downloading(%)  │◀─────────────────────│                      │
   │◀─────────────────│                      │                      │
   │                  │                      │                      │
   │                  │  Save to ~/Documents/models/               │
   │                  │──────────┐                                  │
   │                  │          │                                  │
   │                  │◀─────────┘                                  │
   │                  │                                             │
   │                  │  LlmInference(options:)                     │
   │                  │────────────────────────────────────────────▶│
   │                  │                                             │
   │  state: ready    │  Engine initialized                        │
   │◀─────────────────│◀────────────────────────────────────────────│
   │                  │                                             │
```

### 2. Inference (User Query)

```
┌──────┐    ┌─────────────┐    ┌────────────────┐    ┌──────────────┐    ┌──────────┐
│ User │    │ ContentView │    │ PromptFormatter│    │ ModelManager │    │ MediaPipe│
└──┬───┘    └──────┬──────┘    └───────┬────────┘    └──────┬───────┘    └────┬─────┘
   │               │                   │                    │                 │
   │ "Send email   │                   │                    │                 │
   │  to Bob"      │                   │                    │                 │
   │──────────────▶│                   │                    │                 │
   │               │                   │                    │                 │
   │               │  buildPrompt()    │                    │                 │
   │               │──────────────────▶│                    │                 │
   │               │                   │                    │                 │
   │               │   Formatted       │                    │                 │
   │               │   prompt string   │                    │                 │
   │               │◀──────────────────│                    │                 │
   │               │                   │                    │                 │
   │               │  generate(prompt:)│                    │                 │
   │               │───────────────────────────────────────▶│                 │
   │               │                   │                    │                 │
   │               │                   │                    │ generateResponse│
   │               │                   │                    │────────────────▶│
   │               │                   │                    │                 │
   │               │                   │                    │  Raw output     │
   │               │                   │                    │◀────────────────│
   │               │                   │                    │                 │
   │               │  Raw model output │                    │                 │
   │               │◀──────────────────────────────────────│                 │
   │               │                   │                    │                 │
   │               │ parseFunctionCalls│                    │                 │
   │               │──────────────────▶│                    │                 │
   │               │                   │                    │                 │
   │               │  [ParsedFunction  │                    │                 │
   │               │   Call]           │                    │                 │
   │               │◀──────────────────│                    │                 │
   │               │                   │                    │                 │
   │  send_email(  │                   │                    │                 │
   │   to: "Bob")  │                   │                    │                 │
   │◀──────────────│                   │                    │                 │
   │               │                   │                    │                 │
```

### 3. Prompt Construction

```
┌─────────────────────────────────────────────────────────────────┐
│                    PromptFormatter.buildPrompt()                 │
│                                                                 │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │ <start_of_turn>developer                                  │  │
│  │ Current date and time: 2026-04-03T12:00:00.               │  │
│  │ You are a model that can do function calling...            │  │
│  │                                                            │  │
│  │ <start_function_declaration>                               │  │
│  │ declaration:turn_on_flashlight{...}                        │  │
│  │ <end_function_declaration>                                 │  │
│  │                                                            │  │
│  │ <start_function_declaration>                               │  │
│  │ declaration:send_email{...}                                │  │
│  │ <end_function_declaration>                                 │  │
│  │                                                            │  │
│  │ ... (7 functions total)                                    │  │
│  │ <end_of_turn>                                              │  │
│  ├────────────────────────────────────────────────────────────┤  │
│  │ <start_of_turn>user                                        │  │
│  │ Send an email to Bob about the meeting                     │  │
│  │ <end_of_turn>                                              │  │
│  ├────────────────────────────────────────────────────────────┤  │
│  │ <start_of_turn>model                                       │  │
│  │ ← generation starts here                                   │  │
│  └────────────────────────────────────────────────────────────┘  │
│                              │                                   │
│                              ▼                                   │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │ Model Output:                                              │  │
│  │ <start_function_call>                                      │  │
│  │ call:send_email{                                           │  │
│  │   to:<escape>Bob<escape>,                                  │  │
│  │   subject:<escape>the meeting<escape>                      │  │
│  │ }                                                          │  │
│  │ <end_function_call>                                        │  │
│  └────────────────────────────────────────────────────────────┘  │
│                              │                                   │
│                              ▼                                   │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │ Parsed Result:                                             │  │
│  │ send_email(to: "Bob", subject: "the meeting")              │  │
│  └────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

### 4. File & Component Map

```
ios/
├── project.yml                          ← XcodeGen project spec
├── Podfile                              ← MediaPipe dependency
├── setup.sh                             ← One-command project setup
├── download_model.sh                    ← Pre-download model (~271 MB)
└── FunctionGemmaRunner/
    ├── App.swift                         ← @main entry point
    ├── Info.plist                         ← iOS app config
    ├── ContentView.swift                 ← Chat UI + message handling
    │   ├── ChatMessage                   ← Data model (user/assistant)
    │   ├── ChatBubble                    ← Message bubble view
    │   └── sendMessage()                 ← Orchestrates prompt → inference → parse
    ├── ModelManager.swift                ← Model lifecycle
    │   ├── downloadModelIfNeeded()       ← Fetches from HuggingFace
    │   ├── loadModel()                   ← Initializes MediaPipe engine
    │   ├── generate(prompt:)             ← Single-shot inference
    │   └── generateStreaming(prompt:)     ← Token-by-token streaming
    └── PromptFormatter.swift             ← Prompt construction + parsing
        ├── mobileActions                 ← 7 function definitions
        ├── buildPrompt(userQuery:)       ← Assembles full prompt
        ├── parseFunctionCalls(from:)     ← Extracts structured calls
        └── formatForDisplay(_:)          ← Human-readable output
```

### 5. Integration in Another Swift App

```
┌──────────────────────────────────┐
│        Your Existing App         │
│                                  │
│   1. Add MediaPipe pods          │
│   2. Copy ModelManager.swift     │
│      and PromptFormatter.swift   │
│                                  │
│   ┌────────────────────────┐     │
│   │  Your ViewController   │     │
│   │  or SwiftUI View       │     │
│   │                        │     │
│   │  let mm = ModelManager()     │
│   │  await mm               │     │
│   │    .downloadModelIfNeeded()  │
│   │                        │     │
│   │  let prompt =          │     │
│   │    PromptFormatter     │     │
│   │    .buildPrompt(       │     │
│   │      userQuery: "..."  │     │
│   │    )                   │     │
│   │                        │     │
│   │  let response =        │     │
│   │    await mm            │     │
│   │    .generate(          │     │
│   │      prompt: prompt    │     │
│   │    )                   │     │
│   │                        │     │
│   │  let calls =           │     │
│   │    PromptFormatter     │     │
│   │    .parseFunctionCalls(│     │
│   │      from: response    │     │
│   │    )                   │     │
│   └────────────────────────┘     │
└──────────────────────────────────┘
```
