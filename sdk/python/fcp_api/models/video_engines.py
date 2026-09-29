from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast

if TYPE_CHECKING:
  from ..models.engine_budget import EngineBudget





T = TypeVar("T", bound="VideoEngines")



@_attrs_define
class VideoEngines:
    """ The three budgets of a GPU.

        Attributes:
            encode (EngineBudget):
            decode (EngineBudget):
            compose (EngineBudget):
     """

    encode: EngineBudget
    decode: EngineBudget
    compose: EngineBudget
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.engine_budget import EngineBudget # noqa: PLC0415
        encode = self.encode.to_dict()

        decode = self.decode.to_dict()

        compose = self.compose.to_dict()


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "encode": encode,
            "decode": decode,
            "compose": compose,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.engine_budget import EngineBudget # noqa: PLC0415
        d = dict(src_dict)
        encode = EngineBudget.from_dict(d.pop("encode"))




        decode = EngineBudget.from_dict(d.pop("decode"))




        compose = EngineBudget.from_dict(d.pop("compose"))




        video_engines = cls(
            encode=encode,
            decode=decode,
            compose=compose,
        )


        video_engines.additional_properties = d
        return video_engines

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
