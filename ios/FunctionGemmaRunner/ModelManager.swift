import Foundation
import MediaPipeTasksGenAI

/// Manages downloading the FunctionGemma model from HuggingFace and running inference.
@MainActor
final class ModelManager: ObservableObject {

    enum State: Equatable {
        case idle
        case downloading(progress: Double)
        case loading
        case ready
        case generating
        case error(String)
    }

    @Published var state: State = .idle
    @Published var lastResponse: String = ""

    private var llmInference: LlmInference?

    // HuggingFace model download URL
    private static let modelRepoURL =
        "https://huggingface.co/litert-community/FunctionGemma_270M_Mobile_Actions/resolve/main"
    private static let modelFileName = "functiongemma-270m-ft-mobile-actions-q8.task"

    /// Local directory where the model is stored.
    private var modelDirectory: URL {
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        return docs.appendingPathComponent("models", isDirectory: true)
    }

    /// Full local path to the downloaded model file.
    private var localModelPath: URL {
        modelDirectory.appendingPathComponent(Self.modelFileName)
    }

    /// Whether the model has already been downloaded.
    var isModelDownloaded: Bool {
        FileManager.default.fileExists(atPath: localModelPath.path)
    }

    // MARK: - Download

    /// Downloads the model from HuggingFace if it isn't already present.
    func downloadModelIfNeeded() async {
        if isModelDownloaded {
            await loadModel()
            return
        }

        state = .downloading(progress: 0)

        do {
            try FileManager.default.createDirectory(at: modelDirectory, withIntermediateDirectories: true)

            let downloadURL = URL(string: "\(Self.modelRepoURL)/\(Self.modelFileName)")!
            let delegate = DownloadProgressDelegate { [weak self] progress in
                Task { @MainActor in
                    self?.state = .downloading(progress: progress)
                }
            }

            let (tempURL, response) = try await URLSession.shared.download(
                from: downloadURL,
                delegate: delegate
            )

            guard let httpResponse = response as? HTTPURLResponse,
                  httpResponse.statusCode == 200 else {
                state = .error("Download failed with HTTP status: \((response as? HTTPURLResponse)?.statusCode ?? -1)")
                return
            }

            // Move downloaded file to final location
            if FileManager.default.fileExists(atPath: localModelPath.path) {
                try FileManager.default.removeItem(at: localModelPath)
            }
            try FileManager.default.moveItem(at: tempURL, to: localModelPath)

            await loadModel()
        } catch {
            state = .error("Download failed: \(error.localizedDescription)")
        }
    }

    // MARK: - Load

    /// Loads the model into the LLM inference engine.
    func loadModel() async {
        guard isModelDownloaded else {
            state = .error("Model not found. Please download first.")
            return
        }

        state = .loading

        do {
            let options = LlmInference.Options(modelPath: localModelPath.path)
            options.maxTokens = 512
            options.topk = 40
            options.temperature = 0.0  // Deterministic for function calling
            options.randomSeed = 42

            llmInference = try LlmInference(options: options)
            state = .ready
        } catch {
            state = .error("Failed to load model: \(error.localizedDescription)")
        }
    }

    // MARK: - Inference

    /// Sends a prompt to the model and returns the generated response.
    func generate(prompt: String) async -> String {
        guard let inference = llmInference else {
            state = .error("Model not loaded")
            return ""
        }

        state = .generating

        do {
            let response = try inference.generateResponse(inputText: prompt)
            state = .ready
            lastResponse = response
            return response
        } catch {
            state = .error("Inference failed: \(error.localizedDescription)")
            return ""
        }
    }

    /// Sends a prompt and streams the response token by token.
    func generateStreaming(prompt: String) async -> String {
        guard let inference = llmInference else {
            state = .error("Model not loaded")
            return ""
        }

        state = .generating
        var fullResponse = ""

        do {
            let stream = inference.generateResponseAsync(inputText: prompt)
            for try await partial in stream {
                fullResponse += partial
                lastResponse = fullResponse
            }
            state = .ready
            return fullResponse
        } catch {
            state = .error("Streaming inference failed: \(error.localizedDescription)")
            return fullResponse
        }
    }

    /// Resets the model session.
    func reset() {
        llmInference = nil
        state = .idle
        lastResponse = ""
    }
}

// MARK: - Download Progress Delegate

private final class DownloadProgressDelegate: NSObject, URLSessionDownloadDelegate {
    let onProgress: (Double) -> Void

    init(onProgress: @escaping (Double) -> Void) {
        self.onProgress = onProgress
    }

    func urlSession(
        _ session: URLSession,
        downloadTask: URLSessionDownloadTask,
        didWriteData bytesWritten: Int64,
        totalBytesWritten: Int64,
        totalBytesExpectedToWrite: Int64
    ) {
        guard totalBytesExpectedToWrite > 0 else { return }
        let progress = Double(totalBytesWritten) / Double(totalBytesExpectedToWrite)
        onProgress(progress)
    }

    func urlSession(
        _ session: URLSession,
        downloadTask: URLSessionDownloadTask,
        didFinishDownloadingTo location: URL
    ) {
        // Handled in the async download call
    }
}
