import Foundation

public enum Alkove {
    /// The models' rate, and the rate of every output, in Hz.
    public static let rate = 16000

    /// `near: Alkove.nearGapDb`'s gap: a voice this many dB under the session's main voice is far.
    public static let nearGapDb = 10.0

    static let info: Value = (try? Native.info()) ?? .null

    /// This library's version: the same minor as the web, Python and Android packages means the same core and models.
    public static var version: String { info["version"]?.str ?? "" }
}
