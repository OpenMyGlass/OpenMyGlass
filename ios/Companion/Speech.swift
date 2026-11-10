import Foundation

// Phase 1 has no speech engines: the core is told there are none and carries on without. The real
// ones, SFSpeechRecognizer and AVSpeechSynthesizer, come in phase 2.

final class UnavailableSpeechToText: SpeechToTextEngine {
    func transcribe(audioPath: String, mime: String, locale: NoteLocale) throws -> String {
        throw SpeechFailure.Unavailable
    }
}

final class UnavailableTextToSpeech: TextToSpeechEngine {
    func synthesize(text: String, locale: NoteLocale) throws -> SpeechAudio {
        throw SpeechFailure.Unavailable
    }

    func cancel() {}
}
