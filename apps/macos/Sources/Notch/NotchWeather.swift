import Foundation
import SwiftUI

/// Current city and temperature for the notch's overview. Approximate location from the network (no permission prompt),
/// refreshed at most every 30 minutes and only while the overview is shown.
@MainActor
final class NotchWeather: ObservableObject {
    struct Reading { let city: String; let celsius: Int }

    @Published private(set) var reading: Reading?
    private var fetchedAt = Date.distantPast
    private var loading = false

    func refresh() {
        guard !loading, Date().timeIntervalSince(fetchedAt) > 1800 else { return }
        loading = true
        Task {
            defer { loading = false }
            guard let place = try? await Self.json("https://ipwho.is/"),
                  let lat = place["latitude"] as? Double, let lon = place["longitude"] as? Double,
                  let city = place["city"] as? String,
                  let weather = try? await Self.json("https://api.open-meteo.com/v1/forecast?latitude=\(lat)&longitude=\(lon)&current=temperature_2m"),
                  let current = weather["current"] as? [String: Any], let temp = current["temperature_2m"] as? Double
            else { return }
            reading = Reading(city: city, celsius: Int(temp.rounded()))
            fetchedAt = Date()
        }
    }

    private static func json(_ url: String) async throws -> [String: Any] {
        var request = URLRequest(url: URL(string: url)!)
        request.timeoutInterval = 8
        let (data, _) = try await URLSession.shared.data(for: request)
        return (try JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
    }
}

struct WeatherStrip: View {
    @ObservedObject var weather: NotchWeather

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "cloud.sun").font(.system(size: 12))
            if let reading = weather.reading {
                Text(reading.city).lineLimit(1)
                Text("\(reading.celsius) °C").monospacedDigit()
            }
        }
        .font(.system(size: 11, weight: .semibold))
        .foregroundStyle(.secondary)
        .frame(height: NotchLayout.usageHeight)
        .opacity(weather.reading == nil ? 0 : 1)
        .onAppear { weather.refresh() }
    }
}
