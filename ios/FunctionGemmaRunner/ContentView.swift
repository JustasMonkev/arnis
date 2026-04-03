import SwiftUI

struct ContentView: View {
    @StateObject private var modelManager = ModelManager()
    @State private var userInput = ""
    @State private var chatMessages: [ChatMessage] = []

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                statusBanner

                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 12) {
                            ForEach(chatMessages) { message in
                                ChatBubble(message: message)
                                    .id(message.id)
                            }
                        }
                        .padding()
                    }
                    .onChange(of: chatMessages.count) {
                        if let last = chatMessages.last {
                            withAnimation {
                                proxy.scrollTo(last.id, anchor: .bottom)
                            }
                        }
                    }
                }

                Divider()
                inputBar
            }
            .navigationTitle("FunctionGemma")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    if modelManager.state == .ready || modelManager.state == .idle {
                        Menu {
                            Button("Reset Model") { modelManager.reset() }
                            Button("Clear Chat") { chatMessages.removeAll() }
                        } label: {
                            Image(systemName: "ellipsis.circle")
                        }
                    }
                }
            }
            .task {
                await modelManager.downloadModelIfNeeded()
            }
        }
    }

    // MARK: - Status Banner

    @ViewBuilder
    private var statusBanner: some View {
        switch modelManager.state {
        case .idle:
            EmptyView()
        case .downloading(let progress):
            VStack(spacing: 4) {
                Text("Downloading model...")
                    .font(.caption)
                ProgressView(value: progress)
                    .progressViewStyle(.linear)
                Text("\(Int(progress * 100))%")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
            .padding()
            .background(.ultraThinMaterial)
        case .loading:
            HStack {
                ProgressView()
                Text("Loading model...")
                    .font(.caption)
            }
            .padding()
            .background(.ultraThinMaterial)
        case .ready:
            EmptyView()
        case .generating:
            HStack {
                ProgressView()
                Text("Generating...")
                    .font(.caption)
            }
            .padding(8)
            .background(.ultraThinMaterial)
        case .error(let message):
            VStack(spacing: 8) {
                Label(message, systemImage: "exclamationmark.triangle")
                    .font(.caption)
                    .foregroundStyle(.red)
                Button("Retry") {
                    Task { await modelManager.downloadModelIfNeeded() }
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.small)
            }
            .padding()
            .background(.ultraThinMaterial)
        }
    }

    // MARK: - Input Bar

    private var inputBar: some View {
        HStack(spacing: 8) {
            TextField("Ask something...", text: $userInput, axis: .vertical)
                .textFieldStyle(.plain)
                .lineLimit(1...4)
                .padding(10)
                .background(Color(.systemGray6))
                .clipShape(RoundedRectangle(cornerRadius: 16))
                .disabled(modelManager.state != .ready)

            Button {
                sendMessage()
            } label: {
                Image(systemName: "arrow.up.circle.fill")
                    .font(.title2)
            }
            .disabled(userInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                       || modelManager.state != .ready)
        }
        .padding(.horizontal)
        .padding(.vertical, 8)
        .background(.bar)
    }

    // MARK: - Actions

    private func sendMessage() {
        let query = userInput.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return }

        userInput = ""

        let userMessage = ChatMessage(role: .user, text: query)
        chatMessages.append(userMessage)

        Task {
            let prompt = PromptFormatter.buildPrompt(userQuery: query)
            let rawResponse = await modelManager.generate(prompt: prompt)

            let calls = PromptFormatter.parseFunctionCalls(from: rawResponse)
            let displayText: String
            if calls.isEmpty {
                displayText = rawResponse.isEmpty ? "No response generated." : rawResponse
            } else {
                displayText = PromptFormatter.formatForDisplay(calls)
            }

            let assistantMessage = ChatMessage(
                role: .assistant,
                text: displayText,
                rawResponse: rawResponse
            )
            chatMessages.append(assistantMessage)
        }
    }
}

// MARK: - Chat Models

struct ChatMessage: Identifiable {
    let id = UUID()
    let role: Role
    let text: String
    var rawResponse: String?

    enum Role {
        case user
        case assistant
    }
}

// MARK: - Chat Bubble

struct ChatBubble: View {
    let message: ChatMessage
    @State private var showRaw = false

    var body: some View {
        VStack(alignment: message.role == .user ? .trailing : .leading, spacing: 4) {
            HStack {
                if message.role == .user { Spacer(minLength: 40) }

                VStack(alignment: .leading, spacing: 6) {
                    Text(showRaw ? (message.rawResponse ?? message.text) : message.text)
                        .font(.body)
                        .padding(12)
                        .background(message.role == .user ? Color.blue : Color(.systemGray5))
                        .foregroundStyle(message.role == .user ? .white : .primary)
                        .clipShape(RoundedRectangle(cornerRadius: 16))

                    if message.rawResponse != nil {
                        Button(showRaw ? "Show Parsed" : "Show Raw") {
                            showRaw.toggle()
                        }
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    }
                }

                if message.role == .assistant { Spacer(minLength: 40) }
            }
        }
        .frame(maxWidth: .infinity, alignment: message.role == .user ? .trailing : .leading)
    }
}

// MARK: - Example Queries

extension ContentView {
    static let exampleQueries = [
        "Turn on the flashlight",
        "Send an email to john@example.com about the meeting",
        "Create a calendar event for lunch tomorrow at noon",
        "Show me where the Eiffel Tower is",
        "Add a contact for Jane Doe with phone number 555-1234",
        "Open WiFi settings",
    ]
}

#Preview {
    ContentView()
}
