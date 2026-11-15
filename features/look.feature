Feature: Look and feel
  The page reads well on every screen, respects reduced motion, and stays smooth.

  @look.fonts-from-own-origin
  Scenario: Fonts load from the app's own origin and nowhere else
    When a person opens Rubato
    Then every font comes from Rubato's own origin

  @look.fits-every-width
  Scenario: Nothing overflows at any width, in either colour scheme
    Given a phone, tablet or desktop width, in light or dark
    When a person opens Rubato
    Then nothing is wider than the viewport
    And the panel stays on screen

  @look.reduced-motion
  Scenario: With reduced motion, nothing animates
    Given a person who asked for reduced motion
    When they use the map, its lenses and the journey player
    Then nothing animates
    And moving things show as still marks

  @look.smooth-on-a-large-map
  Scenario: A large map stays smooth
    Given a map of 40 parts and 60 seams, every seam busy
    When a person looks at it with flows moving
    Then no frame takes long enough to stutter
