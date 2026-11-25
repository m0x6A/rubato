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
    Then at most one frame runs long, and not long enough to stutter

  @look.panel-expands
  Scenario: The panel can be widened to read in, and the browser remembers
    Given a window wide enough for the map and the panel side by side
    When a person presses the panel's size button
    Then the panel goes from normal to wide, with the map still beside it, then fills the window
    And the next press brings back the normal width and the map
    And the size is the same after a reload
    And on a narrow window the panel has one size, whatever was stored
