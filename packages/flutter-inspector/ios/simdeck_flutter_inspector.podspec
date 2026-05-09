Pod::Spec.new do |s|
  s.name             = 'simdeck_flutter_inspector'
  s.version          = '0.1.0'
  s.summary          = 'Debug-only Flutter runtime inspector for SimDeck.'
  s.description      = 'Publishes Flutter widget hierarchy metadata to SimDeck during debug sessions.'
  s.homepage         = 'https://github.com/NativeScript/SimDeck'
  s.license          = { :type => 'Apache-2.0' }
  s.author           = { 'SimDeck' => 'support@nativescript.org' }
  s.source           = { :path => '.' }
  s.source_files     = 'Classes/**/*'
  s.dependency 'Flutter'
  s.platform         = :ios, '12.0'
  s.swift_version    = '5.0'
end
