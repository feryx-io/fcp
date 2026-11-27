from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.platform_speech_request_minute_prices_type_0 import PlatformSpeechRequestMinutePricesType0
  from ..models.platform_speech_request_prices_type_0 import PlatformSpeechRequestPricesType0





T = TypeVar("T", bound="PlatformSpeechRequest")



@_attrs_define
class PlatformSpeechRequest:
    """ 
        Attributes:
            held_back (list[str]):
            prices (None | PlatformSpeechRequestPricesType0 | Unset): Prices per million characters by provider; absent
                keeps them, and a
                provider left out has none.
            minute_prices (None | PlatformSpeechRequestMinutePricesType0 | Unset): Prices per minute heard by recognizer;
                absent keeps them.
     """

    held_back: list[str]
    prices: None | PlatformSpeechRequestPricesType0 | Unset = UNSET
    minute_prices: None | PlatformSpeechRequestMinutePricesType0 | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
        from ..models.platform_speech_request_minute_prices_type_0 import PlatformSpeechRequestMinutePricesType0 # noqa: PLC0415
        from ..models.platform_speech_request_prices_type_0 import PlatformSpeechRequestPricesType0 # noqa: PLC0415
        held_back = self.held_back



        prices: dict[str, Any] | None | Unset
        if isinstance(self.prices, Unset):
            prices = UNSET
        elif isinstance(self.prices, PlatformSpeechRequestPricesType0):
            prices = self.prices.to_dict()
        else:
            prices = self.prices

        minute_prices: dict[str, Any] | None | Unset
        if isinstance(self.minute_prices, Unset):
            minute_prices = UNSET
        elif isinstance(self.minute_prices, PlatformSpeechRequestMinutePricesType0):
            minute_prices = self.minute_prices.to_dict()
        else:
            minute_prices = self.minute_prices


        field_dict: dict[str, Any] = {}

        field_dict.update({
            "held_back": held_back,
        })
        if prices is not UNSET:
            field_dict["prices"] = prices
        if minute_prices is not UNSET:
            field_dict["minute_prices"] = minute_prices

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.platform_speech_request_minute_prices_type_0 import PlatformSpeechRequestMinutePricesType0 # noqa: PLC0415
        from ..models.platform_speech_request_prices_type_0 import PlatformSpeechRequestPricesType0 # noqa: PLC0415
        d = dict(src_dict)
        held_back = cast(list[str], d.pop("held_back"))


        def _parse_prices(data: object) -> None | PlatformSpeechRequestPricesType0 | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                prices_type_0 = PlatformSpeechRequestPricesType0.from_dict(data)



                return prices_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(None | PlatformSpeechRequestPricesType0 | Unset, data)

        prices = _parse_prices(d.pop("prices", UNSET))


        def _parse_minute_prices(data: object) -> None | PlatformSpeechRequestMinutePricesType0 | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                minute_prices_type_0 = PlatformSpeechRequestMinutePricesType0.from_dict(data)



                return minute_prices_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(None | PlatformSpeechRequestMinutePricesType0 | Unset, data)

        minute_prices = _parse_minute_prices(d.pop("minute_prices", UNSET))


        platform_speech_request = cls(
            held_back=held_back,
            prices=prices,
            minute_prices=minute_prices,
        )

        return platform_speech_request

