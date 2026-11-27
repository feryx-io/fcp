from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.cap_unit import CapUnit
from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="SpeechCapRequest")



@_attrs_define
class SpeechCapRequest:
    """ A cap to keep. `monthly` null is allowed only on a tenant's platform
    cap: none for this tenant, whatever the default says.

        Attributes:
            monthly (int | None | Unset): How many of `unit` a month (`monthly_characters` is read too).
            unit (CapUnit | None | Unset):
            kind (None | str | Unset): `tenant` (the default: the tenant's own) or `platform` (a platform
                administrator's cap on this tenant).
     """

    monthly: int | None | Unset = UNSET
    unit: CapUnit | None | Unset = UNSET
    kind: None | str | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
        monthly: int | None | Unset
        if isinstance(self.monthly, Unset):
            monthly = UNSET
        else:
            monthly = self.monthly

        unit: None | str | Unset
        if isinstance(self.unit, Unset):
            unit = UNSET
        elif isinstance(self.unit, CapUnit):
            unit = self.unit.value
        else:
            unit = self.unit

        kind: None | str | Unset
        if isinstance(self.kind, Unset):
            kind = UNSET
        else:
            kind = self.kind


        field_dict: dict[str, Any] = {}

        field_dict.update({
        })
        if monthly is not UNSET:
            field_dict["monthly"] = monthly
        if unit is not UNSET:
            field_dict["unit"] = unit
        if kind is not UNSET:
            field_dict["kind"] = kind

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_monthly(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        monthly = _parse_monthly(d.pop("monthly", UNSET))


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


        def _parse_kind(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        kind = _parse_kind(d.pop("kind", UNSET))


        speech_cap_request = cls(
            monthly=monthly,
            unit=unit,
            kind=kind,
        )

        return speech_cap_request

