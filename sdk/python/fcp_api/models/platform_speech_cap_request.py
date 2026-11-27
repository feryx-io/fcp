from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.cap_unit import CapUnit
from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="PlatformSpeechCapRequest")



@_attrs_define
class PlatformSpeechCapRequest:
    """ 
        Attributes:
            monthly (int): How many of `unit` a month (`monthly_characters` is read too).
            unit (CapUnit | None | Unset):
     """

    monthly: int
    unit: CapUnit | None | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
        monthly = self.monthly

        unit: None | str | Unset
        if isinstance(self.unit, Unset):
            unit = UNSET
        elif isinstance(self.unit, CapUnit):
            unit = self.unit.value
        else:
            unit = self.unit


        field_dict: dict[str, Any] = {}

        field_dict.update({
            "monthly": monthly,
        })
        if unit is not UNSET:
            field_dict["unit"] = unit

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        monthly = d.pop("monthly")

        def _parse_unit(data: object) -> CapUnit | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                unit_type_0 = CapUnit(data)



                return unit_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(CapUnit | None | Unset, data)

        unit = _parse_unit(d.pop("unit", UNSET))


        platform_speech_cap_request = cls(
            monthly=monthly,
            unit=unit,
        )

        return platform_speech_cap_request

