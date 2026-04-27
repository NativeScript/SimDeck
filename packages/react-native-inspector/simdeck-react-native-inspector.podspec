require "json"

package = JSON.parse(File.read(File.join(__dir__, "package.json")))

Pod::Spec.new do |s|
  s.name         = "simdeck-react-native-inspector"
  s.version      = package["version"]
  s.summary      = package["description"]
  s.license      = package["license"]
  s.homepage     = "https://github.com/DjDeveloperr/SimDeck"
  s.authors      = { "SimDeck contributors" => "simdeck@example.invalid" }
  s.platforms    = { :ios => "12.4" }
  s.source       = { :git => "https://github.com/DjDeveloperr/SimDeck.git", :tag => "v#{s.version}" }
  s.source_files = "ios/**/*.{h,m,mm}"
  s.dependency "React-Core"
end
