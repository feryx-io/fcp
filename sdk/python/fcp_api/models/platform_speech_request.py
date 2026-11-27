from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
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
     """

    held_back: list[str]
    prices: None | PlatformSpeechRequestPricesType0 | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
        from ..models.platform_speech_request_prices_type_0 import PlatformSpeechRequestPricesType0 # noqa: PLC0415
        held_back = self.held_back



        prices: dict[str, Any] | None | Unset
        if isinstance(self.prices, Unset):
            prices = UNSET
        elif isinstance(self.prices, PlatformSpeechRequestPricesType0):
            prices = self.prices.to_dict()
        else:
            prices = self.prices


        field_dict: dict[str, Any] = {}

        field_dict.update({
            "held_back": held_back,
        })
        if prices is not UNSET:
            field_dict["prices"] = prices

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
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


        platform_speech_request = cls(
            held_back=held_back,
            prices=prices,
        )

        return platform_speech_request

