import Foundation

/// Formats prompts for the FunctionGemma mobile-actions model.
///
/// The model expects a specific chat template with function declarations in the developer turn,
/// and user queries in the user turn. It responds with structured function calls like:
/// `call:function_name{arg1:<escape>value<escape>,arg2:<escape>value<escape>}`
enum PromptFormatter {

    // MARK: - Function Definitions

    struct FunctionParameter {
        let name: String
        let type: String
        let description: String
        let required: Bool
    }

    struct FunctionDefinition {
        let name: String
        let description: String
        let parameters: [FunctionParameter]
    }

    /// The set of mobile action functions the model was fine-tuned on.
    static let mobileActions: [FunctionDefinition] = [
        FunctionDefinition(
            name: "turn_on_flashlight",
            description: "Turns on the device flashlight.",
            parameters: []
        ),
        FunctionDefinition(
            name: "turn_off_flashlight",
            description: "Turns off the device flashlight.",
            parameters: []
        ),
        FunctionDefinition(
            name: "send_email",
            description: "Sends an email to the specified recipient.",
            parameters: [
                .init(name: "to", type: "STRING", description: "The email address of the recipient.", required: true),
                .init(name: "subject", type: "STRING", description: "The subject of the email.", required: true),
                .init(name: "body", type: "STRING", description: "The body text of the email.", required: false),
            ]
        ),
        FunctionDefinition(
            name: "create_contact",
            description: "Creates a new contact.",
            parameters: [
                .init(name: "first_name", type: "STRING", description: "The first name of the contact.", required: true),
                .init(name: "last_name", type: "STRING", description: "The last name of the contact.", required: true),
                .init(name: "email", type: "STRING", description: "The email address of the contact.", required: false),
                .init(name: "phone_number", type: "STRING", description: "The phone number of the contact.", required: false),
            ]
        ),
        FunctionDefinition(
            name: "create_calendar_event",
            description: "Creates a new calendar event.",
            parameters: [
                .init(name: "title", type: "STRING", description: "The title of the event.", required: true),
                .init(name: "datetime", type: "STRING", description: "Event date/time in YYYY-MM-DDTHH:MM:SS format.", required: true),
            ]
        ),
        FunctionDefinition(
            name: "show_map",
            description: "Shows a location on the map.",
            parameters: [
                .init(name: "query", type: "STRING", description: "The location to search for.", required: true),
            ]
        ),
        FunctionDefinition(
            name: "open_wifi_settings",
            description: "Opens the Wi-Fi settings page.",
            parameters: []
        ),
    ]

    // MARK: - Formatting

    /// Formats a single function definition into the model's expected XML-like declaration format.
    static func formatFunctionDeclaration(_ fn: FunctionDefinition) -> String {
        var decl = "<start_function_declaration>\n"
        decl += "declaration:\(fn.name){\n"
        decl += "  description:<escape>\(fn.description)<escape>"

        if !fn.parameters.isEmpty {
            decl += ",\n  parameters:{\n"
            decl += "    properties:{\n"

            for (i, param) in fn.parameters.enumerated() {
                decl += "      \(param.name):{type:<escape>\(param.type)<escape>}"
                if i < fn.parameters.count - 1 { decl += "," }
                decl += "\n"
            }

            decl += "    },\n"

            let requiredParams = fn.parameters.filter(\.required).map { "<escape>\(($0).name)<escape>" }
            decl += "    required:[\(requiredParams.joined(separator: ","))],\n"
            decl += "    type:<escape>OBJECT<escape>\n"
            decl += "  }"
        }

        decl += "\n}\n<end_function_declaration>"
        return decl
    }

    /// Builds the full prompt with system context, function declarations, and user query.
    static func buildPrompt(userQuery: String) -> String {
        let dateFormatter = ISO8601DateFormatter()
        dateFormatter.formatOptions = [.withFullDate, .withFullTime, .withDashSeparatorInDate, .withColonSeparatorInTime]
        let currentDateTime = dateFormatter.string(from: Date())
            .replacingOccurrences(of: "Z", with: "")

        // Developer turn with function declarations
        var prompt = "<start_of_turn>developer\n"
        prompt += "Current date and time given in YYYY-MM-DDTHH:MM:SS format: \(currentDateTime). "
        prompt += "You are a model that can do function calling with the following functions\n\n"

        for fn in mobileActions {
            prompt += formatFunctionDeclaration(fn)
            prompt += "\n\n"
        }

        prompt += "<end_of_turn>\n"

        // User turn
        prompt += "<start_of_turn>user\n"
        prompt += userQuery
        prompt += "<end_of_turn>\n"

        // Model turn (generation prompt)
        prompt += "<start_of_turn>model\n"

        return prompt
    }

    // MARK: - Response Parsing

    /// A parsed function call from the model's output.
    struct ParsedFunctionCall {
        let functionName: String
        let arguments: [(key: String, value: String)]
    }

    /// Parses the model output into structured function calls.
    ///
    /// The model outputs in the format:
    /// `<start_function_call>call:function_name{arg1:<escape>val1<escape>,arg2:<escape>val2<escape>}<end_function_call>`
    static func parseFunctionCalls(from response: String) -> [ParsedFunctionCall] {
        var calls: [ParsedFunctionCall] = []

        let pattern = #"call:(\w+)\{([^}]*)\}"#
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return calls }

        let nsRange = NSRange(response.startIndex..., in: response)
        let matches = regex.matches(in: response, range: nsRange)

        for match in matches {
            guard match.numberOfRanges >= 3,
                  let nameRange = Range(match.range(at: 1), in: response),
                  let argsRange = Range(match.range(at: 2), in: response) else { continue }

            let functionName = String(response[nameRange])
            let argsString = String(response[argsRange])

            // Parse arguments: key:<escape>value<escape>,key2:<escape>value2<escape>
            var arguments: [(key: String, value: String)] = []
            let argParts = argsString.components(separatedBy: "<escape>,")

            for part in argParts where !part.isEmpty {
                let cleaned = part.replacingOccurrences(of: "<escape>", with: "")
                let keyValue = cleaned.split(separator: ":", maxSplits: 1)
                if keyValue.count == 2 {
                    arguments.append((
                        key: String(keyValue[0]).trimmingCharacters(in: .whitespaces),
                        value: String(keyValue[1]).trimmingCharacters(in: .whitespaces)
                    ))
                }
            }

            calls.append(ParsedFunctionCall(functionName: functionName, arguments: arguments))
        }

        return calls
    }

    /// Formats parsed function calls into a human-readable string.
    static func formatForDisplay(_ calls: [ParsedFunctionCall]) -> String {
        if calls.isEmpty { return "No function calls detected." }

        return calls.map { call in
            var result = "\(call.functionName)("
            let args = call.arguments.map { "\($0.key): \"\($0.value)\"" }
            result += args.joined(separator: ", ")
            result += ")"
            return result
        }.joined(separator: "\n")
    }
}
