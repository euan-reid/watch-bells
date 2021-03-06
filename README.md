# Watch Bells

> Computer time for seafarers

[![pipeline status](https://gitlab.com/euan/watch-bells/badges/master/pipeline.svg)](https://gitlab.com/euan/watch-bells/commits/master)
[![test coverage](https://gitlab.com/euan/watch-bells/badges/master/coverage.svg)](https://gitlab.com/euan/watch-bells/commits/master)

Watch Bells make your computer's time telling a little more nautical

## Installation

Download the latest release binary, put it somewhere that gets automatically started, and you're done

## Development setup

You will need VS Code, Git, and Docker installed; and VS Code Remote - Containers set up.

With that setup in place, use Remote Containers: Clone Repository in Container Volume and clone the repo. Everything else should be automatic.

## Release History

* 0.4.1
	* Fix for Nore adjustments
* 0.4.0
	* Bell icon
	* Fix for dog watch name display (correctly show spaces instead of underscores, e.g. first dog vs first_dog)
* 0.3.0
	* Bundle sound files, allowing offline usage and avoiding load on freesound.org
* 0.2.0
	* Add mute toggle and hover text for current time
* 0.1.1
	* Correct off-by-one error in bell count
* 0.1.0
	* Initial release

## Acknowledgements

The ship's bell chimes are public domain and come from Sojan on freesound.org - thank you to them

## Maintainers

Euan Reid – [@EuanReid](https://twitter.com/EuanReid)

## Contributing

1. Fork the repo (<https://gitlab.com/euan/watch-bells/forks/new>)
2. Create a feature branch (`git checkout -b feature-stuffed-crust`)
3. Make some changes (`git commit -am 'Added stuffed crust'`)
4. Push to your repo (`git push origin feature-stuffed-crust`)
5. Create a new Merge Request (<https://gitlab.com/euan/watch-bells/merge_requests/new>)
